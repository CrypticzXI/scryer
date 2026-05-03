use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Duration, Utc};
use clap::{Args, Parser, Subcommand, ValueEnum};
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::ffi::OsStr;
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{ChildStderr, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::thread;
use tempfile::NamedTempFile;
use toml::Value as TomlValue;
use toml_edit::{DocumentMut, value};

mod corpus;
mod profile;
mod release_parser;
mod release_parser_compare;
mod seed;

const BLUE: &str = "\x1b[0;34m";
const GREEN: &str = "\x1b[0;32m";
const YELLOW: &str = "\x1b[1;33m";
const BOLD: &str = "\x1b[1m";
const RESET: &str = "\x1b[0m";
const PLUGIN_SDK_PACKAGE: &str = "scryer-plugin-sdk";
const PLUGIN_SDK_TAG_PREFIX: &str = "plugin-sdk-v";
const PLUGIN_SDK_MANIFEST: &str = "crates/scryer-plugin-sdk/Cargo.toml";
const PLUGIN_SDK_LIB: &str = "crates/scryer-plugin-sdk/src/lib.rs";
const SCRYER_PROD_PACKAGES: &[&str] = &[
    "scryer",
    "scryer-application",
    "scryer-domain",
    "scryer-infrastructure",
    "scryer-interface",
    "scryer-mediainfo",
    "scryer-plugins",
    "scryer-release-parser",
    "scryer-rules",
];
const BUILTIN_PLUGIN_ARTIFACTS: &[&str] = &[
    "crates/scryer-plugins/builtins/animetosho_indexer.wasm",
    "crates/scryer-plugins/builtins/jimaku_subtitle_provider.wasm",
    "crates/scryer-plugins/builtins/newznab_indexer.wasm",
    "crates/scryer-plugins/builtins/nzbgeek_indexer.wasm",
    "crates/scryer-plugins/builtins/torznab_indexer.wasm",
];
const RELEASE_DRY_RUN_CACHE_FILE: &str = "tmp/xtask-release-dry-run.json";
const RELEASE_DRY_RUN_BUILTINS_DIR: &str = "tmp/xtask-release-dry-run-builtins";
const OFFICIAL_PLUGIN_CATALOG_URL: &str =
    "https://github.com/scryer-media/scryer-plugins/releases/download/catalog%2Fv2/catalog-v2.json";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask has a repo root parent")
        .to_path_buf()
}

#[derive(Parser)]
#[command(name = "cargo xtask")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Release(ReleaseArgs),
    Sdk(SdkArgs),
    ValidateTrashGuides,
    Ci(CiArgs),
    Stack(StackArgs),
    Nzbget(NzbgetArgs),
    Seed(SeedArgs),
    Profile(ProfileArgs),
    Corpus(CorpusArgs),
}

#[derive(Args)]
struct ReleaseArgs {
    #[arg(long, conflicts_with_all = ["minor", "patch", "version"])]
    major: bool,
    #[arg(long, conflicts_with_all = ["major", "patch", "version"])]
    minor: bool,
    #[arg(long, conflicts_with_all = ["major", "minor", "version"])]
    patch: bool,
    #[arg(long)]
    dry_run: bool,
    version: Option<String>,
}

#[derive(Args)]
struct SdkArgs {
    #[command(subcommand)]
    command: SdkCommand,
}

#[derive(Subcommand)]
enum SdkCommand {
    Release(SdkReleaseArgs),
}

#[derive(Args)]
struct SdkReleaseArgs {
    version: String,
    #[arg(long)]
    dry_run: bool,
}

#[derive(Args)]
struct CiArgs {
    #[command(subcommand)]
    command: CiCommand,
}

#[derive(Subcommand)]
enum CiCommand {
    Clippy(ClippyArgs),
}

#[derive(Args)]
struct ClippyArgs {
    #[arg(long)]
    linux_only: bool,
}

#[derive(Args)]
struct StackArgs {
    #[command(subcommand)]
    command: StackCommand,
}

#[derive(Subcommand)]
enum StackCommand {
    Up(StackUpArgs),
    Down(StackDownArgs),
    Logs(StackLogsArgs),
    Restart(StackRestartArgs),
}

#[derive(Args)]
struct StackUpArgs {
    #[arg(
        long,
        help = "Also run the one-shot seed container after the stack is up"
    )]
    seed: bool,
}

#[derive(Args)]
struct StackRestartArgs {
    #[arg(
        long,
        help = "Also run the one-shot seed container after the stack is back up"
    )]
    seed: bool,
}

#[derive(Args)]
struct StackDownArgs {
    #[arg(long)]
    all: bool,
}

#[derive(Args)]
struct StackLogsArgs {
    service: Option<String>,
}

#[derive(Args)]
struct NzbgetArgs {
    #[command(subcommand)]
    command: NzbgetCommand,
}

#[derive(Subcommand)]
enum NzbgetCommand {
    Up,
    Down,
}

#[derive(Args)]
struct SeedArgs {
    #[command(subcommand)]
    command: SeedCommand,
}

#[derive(Subcommand)]
enum SeedCommand {
    Dev(SeedDevArgs),
}

#[derive(Args)]
struct SeedDevArgs {
    #[arg(long)]
    file: Option<PathBuf>,
}

#[derive(Args)]
struct ProfileArgs {
    #[command(subcommand)]
    command: ProfileCommand,
}

#[derive(Subcommand)]
enum ProfileCommand {
    Hotpaths(ProfileHotpathsArgs),
}

#[derive(Args)]
struct ProfileHotpathsArgs {
    duration_seconds: Option<String>,
    interval_seconds: Option<String>,
}

#[derive(Args)]
struct CorpusArgs {
    #[command(subcommand)]
    command: CorpusCommand,
}

#[derive(Subcommand)]
enum CorpusCommand {
    ReleaseParser(ReleaseParserCorpusArgs),
    ReleaseParserEval(ReleaseParserEvalArgs),
    GuessitEval(ReleaseParserEvalArgs),
    SonarrEval(ReleaseParserEvalArgs),
    RadarrEval(ReleaseParserEvalArgs),
}

#[derive(Args, Clone)]
pub(crate) struct ReleaseParserCorpusArgs {
    #[arg(long, default_value_t = 1000)]
    total: usize,
    #[arg(long)]
    output_dir: Option<PathBuf>,
    #[arg(long, default_value_t = 8)]
    nzbgeek_movie_pages: usize,
    #[arg(long, default_value_t = 8)]
    nzbgeek_series_pages: usize,
    #[arg(long, default_value_t = 6)]
    nzbgeek_anime_pages: usize,
    #[arg(long, default_value_t = 12)]
    animetosho_pages: usize,
    #[arg(long, default_value_t = 4)]
    max_per_title: usize,
}

#[derive(Args, Clone)]
pub(crate) struct ReleaseParserEvalArgs {
    #[arg(long)]
    input: Option<PathBuf>,
    #[arg(long)]
    output_dir: Option<PathBuf>,
    #[arg(long, default_value_t = 50)]
    max_mismatches: usize,
}

#[derive(Copy, Clone, Eq, PartialEq, ValueEnum)]
enum VersionBump {
    Patch,
    Minor,
    Major,
}

#[derive(Clone)]
struct TaskContext {
    repo_root: PathBuf,
    rtk_available: bool,
}

impl TaskContext {
    fn new() -> Self {
        let rtk_available = match validated_release_rtk_available() {
            Ok(available) => available,
            Err(err) => {
                eprintln!(
                    "{YELLOW}warning:{RESET} {err}. Falling back to direct command execution."
                );
                false
            }
        };
        Self {
            repo_root: repo_root(),
            rtk_available,
        }
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.repo_root.join(relative)
    }

    fn command(&self, program: impl AsRef<OsStr>) -> Command {
        Command::new(program)
    }

    fn command_in(&self, program: impl AsRef<OsStr>, cwd: &Path) -> Command {
        let mut command = Command::new(program);
        command.current_dir(cwd);
        command
    }

    fn release_command(&self, program: impl AsRef<OsStr>) -> Command {
        self.release_command_impl(program, None)
    }

    fn release_command_in(&self, program: impl AsRef<OsStr>, cwd: &Path) -> Command {
        self.release_command_impl(program, Some(cwd))
    }

    fn release_command_impl(&self, program: impl AsRef<OsStr>, cwd: Option<&Path>) -> Command {
        let program = program.as_ref();
        let mut command = if self.rtk_available {
            let mut command = Command::new("rtk");
            command.arg(program);
            command
        } else {
            Command::new(program)
        };
        if let Some(cwd) = cwd {
            command.current_dir(cwd);
        }
        command
    }
}

#[derive(Debug, Deserialize)]
struct GhRelease {
    #[serde(rename = "tagName")]
    tag_name: String,
    #[serde(rename = "publishedAt")]
    published_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ReleaseDryRunCache {
    success: bool,
    created_at: String,
    git_commit: String,
    branch: String,
    release_args: String,
    latest_tag_seen: Option<String>,
    next_version: String,
    tag_name: String,
    catalog_url: String,
    catalog_checksum_sha256: Option<String>,
    validated_steps: Vec<String>,
    cached_builtins_dir: Option<String>,
    failure_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReleaseDryRunExpectations<'a> {
    git_commit: &'a str,
    release_args: &'a str,
    latest_tag_seen: Option<&'a str>,
    next_version: &'a str,
    tag_name: &'a str,
    catalog_checksum_sha256: &'a str,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let ctx = TaskContext::new();

    match cli.command {
        Commands::Release(args) => run_release(&ctx, args),
        Commands::Sdk(args) => match args.command {
            SdkCommand::Release(args) => run_sdk_release(&ctx, args),
        },
        Commands::ValidateTrashGuides => run_validate_trash_guides(&ctx),
        Commands::Ci(args) => match args.command {
            CiCommand::Clippy(args) => run_clippy_ci(&ctx, args),
        },
        Commands::Stack(args) => match args.command {
            StackCommand::Up(args) => stack_up(&ctx, args),
            StackCommand::Down(args) => stack_down(&ctx, args),
            StackCommand::Logs(args) => stack_logs(&ctx, args),
            StackCommand::Restart(args) => stack_restart(&ctx, args),
        },
        Commands::Nzbget(args) => match args.command {
            NzbgetCommand::Up => nzbget_up(&ctx),
            NzbgetCommand::Down => nzbget_down(&ctx),
        },
        Commands::Seed(args) => match args.command {
            SeedCommand::Dev(args) => seed_dev(&ctx, args),
        },
        Commands::Profile(args) => match args.command {
            ProfileCommand::Hotpaths(args) => profile_hotpaths(&ctx, args),
        },
        Commands::Corpus(args) => match args.command {
            CorpusCommand::ReleaseParser(args) => corpus::run_release_parser(&ctx, args),
            CorpusCommand::ReleaseParserEval(args) => release_parser::run_eval(&ctx, args),
            CorpusCommand::GuessitEval(args) => {
                release_parser_compare::run_guessit_eval(&ctx, args)
            }
            CorpusCommand::SonarrEval(args) => release_parser_compare::run_sonarr_eval(&ctx, args),
            CorpusCommand::RadarrEval(args) => release_parser_compare::run_radarr_eval(&ctx, args),
        },
    }
}

fn seed_dev(ctx: &TaskContext, args: SeedDevArgs) -> Result<()> {
    seed::run(ctx, args)
}

fn step(message: impl AsRef<str>) {
    println!("\n{BLUE}{BOLD}▶  {}{RESET}", message.as_ref());
}

fn ok(message: impl AsRef<str>) {
    println!("   {GREEN}✓  {}{RESET}", message.as_ref());
}

fn warn(message: impl AsRef<str>) {
    eprintln!("   {YELLOW}⚠  {}{RESET}", message.as_ref());
}

fn prefixed_step(prefix: &str, message: impl AsRef<str>) {
    println!("{prefix}{BLUE}{BOLD}▶  {}{RESET}", message.as_ref());
}

fn prefixed_ok(prefix: &str, message: impl AsRef<str>) {
    println!("{prefix}{GREEN}✓  {}{RESET}", message.as_ref());
}

fn require_command(command: &str) -> Result<()> {
    if command_available(command)? {
        Ok(())
    } else {
        bail!("{command} is required")
    }
}

fn command_available(command: &str) -> Result<bool> {
    let status = Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {command} >/dev/null 2>&1"))
        .status()?;
    Ok(status.success())
}

fn rustup_toolchain_from_file(path: &Path) -> Result<Option<String>> {
    if !path.is_file() {
        return Ok(None);
    }

    let document = toml::from_str::<TomlValue>(
        &fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?,
    )
    .with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(document
        .get("toolchain")
        .and_then(|toolchain| toolchain.get("channel"))
        .and_then(TomlValue::as_str)
        .map(ToOwned::to_owned))
}

fn repo_rustup_toolchain(repo_root: &Path) -> Result<Option<String>> {
    if !command_available("rustup")? {
        return Ok(None);
    }

    if let Some(toolchain) = rustup_toolchain_from_file(&repo_root.join("rust-toolchain.toml"))? {
        return Ok(Some(toolchain));
    }

    let mut command = Command::new("rustup");
    command.current_dir(repo_root);
    command.args(["show", "active-toolchain"]);
    Ok(run_capture(&mut command)?
        .split_whitespace()
        .next()
        .map(str::to_string))
}

fn repo_release_cargo_command_in(ctx: &TaskContext, cwd: &Path) -> Result<Command> {
    if let Some(toolchain) = repo_rustup_toolchain(cwd)? {
        let mut command = ctx.release_command_in("rustup", cwd);
        command.args(["run", toolchain.as_str(), "cargo"]);
        return Ok(command);
    }

    Ok(ctx.release_command_in("cargo", cwd))
}

fn validated_release_rtk_available() -> Result<bool> {
    if !command_available("rtk")? {
        return Ok(false);
    }

    let mut help_command = Command::new("rtk");
    help_command.arg("--help");
    let help = run_capture(&mut help_command).context("failed to inspect `rtk --help`")?;

    let looks_like_rust_token_killer = help.contains("filter and summarize system outputs")
        && help.contains("gain")
        && help.contains("proxy");

    if looks_like_rust_token_killer {
        Ok(true)
    } else {
        Err(anyhow!(
            "`rtk` was found on PATH, but it does not appear to be Rust Token Killer"
        ))
    }
}

fn run_status(command: &mut Command) -> Result<ExitStatus> {
    Ok(command.status()?)
}

fn run_checked(command: &mut Command) -> Result<()> {
    let debug = format!("{command:?}");
    let status = run_status(command)?;
    if !status.success() {
        bail!("command failed: {debug}");
    }
    Ok(())
}

fn run_capture(command: &mut Command) -> Result<String> {
    let debug = format!("{command:?}");
    let output = command.output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("command failed: {debug}\n{stderr}");
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn run_streaming(command: &mut Command, prefix: &'static str) -> Result<()> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdout_thread = stdout.map(|stdout| pipe_reader(stdout, prefix, false));
    let stderr_thread = stderr.map(|stderr| pipe_reader_stderr(stderr, prefix));
    let status = child.wait()?;
    if let Some(thread) = stdout_thread {
        let _ = thread.join();
    }
    if let Some(thread) = stderr_thread {
        let _ = thread.join();
    }
    if !status.success() {
        bail!("command failed: {command:?}");
    }
    Ok(())
}

fn pipe_reader(stream: ChildStdout, prefix: &'static str, stderr: bool) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let reader = BufReader::new(stream);
        for line in reader.lines().map_while(Result::ok) {
            if stderr {
                eprintln!("{prefix}{line}");
            } else {
                println!("{prefix}{line}");
            }
        }
    })
}

fn pipe_reader_stderr(stream: ChildStderr, prefix: &'static str) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let reader = BufReader::new(stream);
        for line in reader.lines().map_while(Result::ok) {
            eprintln!("{prefix}{line}");
        }
    })
}

fn git_capture(ctx: &TaskContext, args: &[&str]) -> Result<String> {
    let mut command = ctx.command_in("git", &ctx.repo_root);
    command.args(args);
    run_capture(&mut command)
}

fn git_status_porcelain(ctx: &TaskContext) -> Result<String> {
    git_capture(ctx, &["status", "--porcelain"])
}

fn latest_prefixed_tag(ctx: &TaskContext, prefix: &str) -> Result<Option<String>> {
    let tags = git_capture(ctx, &["tag", "--sort=-version:refname"])?;
    Ok(tags
        .lines()
        .find(|line| line.starts_with(prefix))
        .map(ToOwned::to_owned))
}

fn current_branch(ctx: &TaskContext) -> Result<String> {
    git_capture(ctx, &["rev-parse", "--abbrev-ref", "HEAD"]).map(|value| value.trim().to_string())
}

fn current_head_commit(ctx: &TaskContext) -> Result<String> {
    git_capture(ctx, &["rev-parse", "HEAD"]).map(|value| value.trim().to_string())
}

fn prompt_continue_if_dirty(ctx: &TaskContext) -> Result<()> {
    let status = git_status_porcelain(ctx)?;
    if status.trim().is_empty() {
        return Ok(());
    }

    warn("Working tree has uncommitted changes:");
    for line in status.lines() {
        eprintln!("     {line}");
    }
    eprint!("\n   Continue anyway? [y/N] ");
    io::stderr().flush()?;

    let mut response = String::new();
    io::stdin().read_line(&mut response)?;
    let response = response.trim();
    if !matches!(response, "y" | "Y") {
        bail!("aborted");
    }
    Ok(())
}

fn release_args_signature(explicit: Option<&Version>, bump: VersionBump) -> String {
    explicit.map_or_else(
        || format!("bump:{}", version_bump_label(bump)),
        |version| format!("version:{version}"),
    )
}

fn version_bump_label(bump: VersionBump) -> &'static str {
    match bump {
        VersionBump::Patch => "patch",
        VersionBump::Minor => "minor",
        VersionBump::Major => "major",
    }
}

fn parse_bump(args: &ReleaseArgs) -> Result<(VersionBump, Option<Version>)> {
    let explicit = match &args.version {
        Some(version) => Some(Version::parse(version.trim_start_matches('v'))?),
        None => None,
    };
    let bump = if args.major {
        VersionBump::Major
    } else if args.minor {
        VersionBump::Minor
    } else {
        VersionBump::Patch
    };
    Ok((bump, explicit))
}

fn release_dry_run_cache_path(ctx: &TaskContext) -> PathBuf {
    ctx.path(RELEASE_DRY_RUN_CACHE_FILE)
}

fn release_dry_run_builtins_root(ctx: &TaskContext) -> PathBuf {
    ctx.path(RELEASE_DRY_RUN_BUILTINS_DIR)
}

fn release_dry_run_cache_fingerprint(
    git_commit: &str,
    release_args: &str,
    latest_tag_seen: Option<&str>,
    next_version: &Version,
    tag_name: &str,
) -> String {
    sha256_hex(
        format!(
            "{}\n{}\n{}\n{}\n{}",
            git_commit,
            release_args,
            latest_tag_seen.unwrap_or(""),
            next_version,
            tag_name
        )
        .as_bytes(),
    )
}

fn release_dry_run_cache_dir(
    ctx: &TaskContext,
    git_commit: &str,
    release_args: &str,
    latest_tag_seen: Option<&str>,
    next_version: &Version,
    tag_name: &str,
) -> PathBuf {
    release_dry_run_builtins_root(ctx).join(release_dry_run_cache_fingerprint(
        git_commit,
        release_args,
        latest_tag_seen,
        next_version,
        tag_name,
    ))
}

fn relative_to_repo_root(ctx: &TaskContext, path: &Path) -> Result<String> {
    path.strip_prefix(&ctx.repo_root)
        .with_context(|| format!("{} is not under repo root", path.display()))
        .map(|relative| relative.to_string_lossy().into_owned())
}

fn clear_release_dry_run_cache(ctx: &TaskContext) -> Result<()> {
    let cache_path = release_dry_run_cache_path(ctx);
    if cache_path.exists() {
        fs::remove_file(&cache_path)
            .with_context(|| format!("failed to remove {}", cache_path.display()))?;
    }

    let builtins_root = release_dry_run_builtins_root(ctx);
    if builtins_root.exists() {
        fs::remove_dir_all(&builtins_root)
            .with_context(|| format!("failed to remove {}", builtins_root.display()))?;
    }

    Ok(())
}

fn write_release_dry_run_cache(ctx: &TaskContext, cache: &ReleaseDryRunCache) -> Result<()> {
    let path = release_dry_run_cache_path(ctx);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(&path, serde_json::to_string_pretty(cache)? + "\n")
        .with_context(|| format!("failed to write {}", path.display()))
}

fn load_release_dry_run_cache(ctx: &TaskContext) -> Result<ReleaseDryRunCache> {
    let path = release_dry_run_cache_path(ctx);
    let raw =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_str(&raw).with_context(|| format!("failed to parse {}", path.display()))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join("")
}

fn fetch_catalog_checksum(url: &str) -> Result<String> {
    let response = reqwest::blocking::get(url)
        .with_context(|| format!("failed to fetch published plugin catalog from {url}"))?
        .error_for_status()
        .with_context(|| format!("published plugin catalog returned error status for {url}"))?;
    let body = response
        .bytes()
        .context("failed to read published plugin catalog body")?;
    Ok(sha256_hex(&body))
}

fn cache_builtin_artifacts(cache_dir: &Path, builtins: &[PathBuf]) -> Result<()> {
    if cache_dir.exists() {
        fs::remove_dir_all(cache_dir)
            .with_context(|| format!("failed to clear {}", cache_dir.display()))?;
    }
    fs::create_dir_all(cache_dir)
        .with_context(|| format!("failed to create {}", cache_dir.display()))?;

    for built_wasm in builtins {
        let file_name = built_wasm
            .file_name()
            .ok_or_else(|| anyhow!("missing builtin file name for {}", built_wasm.display()))?;
        let cached = cache_dir.join(file_name);
        fs::copy(built_wasm, &cached).with_context(|| {
            format!(
                "failed to cache builtin {} to {}",
                built_wasm.display(),
                cached.display()
            )
        })?;
    }

    Ok(())
}

fn builtin_cache_complete(cache_dir: &Path, builtins: &[PathBuf]) -> bool {
    cache_dir.is_dir()
        && builtins.iter().all(|built_wasm| {
            built_wasm
                .file_name()
                .map(|file_name| cache_dir.join(file_name).is_file())
                .unwrap_or(false)
        })
}

fn restore_builtin_artifacts_from_cache(cache_dir: &Path, builtins: &[PathBuf]) -> Result<()> {
    if !builtin_cache_complete(cache_dir, builtins) {
        bail!(
            "cached builtin artifacts are missing or incomplete under {}",
            cache_dir.display()
        );
    }

    for output_wasm in builtins {
        let file_name = output_wasm
            .file_name()
            .ok_or_else(|| anyhow!("missing builtin file name for {}", output_wasm.display()))?;
        let cached = cache_dir.join(file_name);
        fs::copy(&cached, output_wasm).with_context(|| {
            format!(
                "failed to restore cached builtin {} to {}",
                cached.display(),
                output_wasm.display()
            )
        })?;
    }

    Ok(())
}

fn release_dry_run_cache_rejection_reason(
    cache: &ReleaseDryRunCache,
    expected: &ReleaseDryRunExpectations<'_>,
    builtins_present: bool,
) -> Option<String> {
    if !cache.success {
        return Some("previous dry run did not complete successfully".to_string());
    }
    if cache.git_commit != expected.git_commit {
        return Some("HEAD commit changed since dry run".to_string());
    }
    if cache.release_args != expected.release_args {
        return Some("release arguments changed since dry run".to_string());
    }
    if cache.latest_tag_seen.as_deref() != expected.latest_tag_seen {
        return Some("latest release tag changed since dry run".to_string());
    }
    if cache.next_version != expected.next_version {
        return Some("computed next version changed since dry run".to_string());
    }
    if cache.tag_name != expected.tag_name {
        return Some("computed release tag changed since dry run".to_string());
    }
    if cache.catalog_checksum_sha256.as_deref() != Some(expected.catalog_checksum_sha256) {
        return Some("published plugin catalog checksum changed since dry run".to_string());
    }
    if !builtins_present {
        return Some("cached builtin artifacts are missing".to_string());
    }
    None
}

fn next_version(current: &Version, bump: VersionBump) -> Version {
    let mut next = current.clone();
    match bump {
        VersionBump::Patch => {
            next.patch += 1;
        }
        VersionBump::Minor => {
            next.minor += 1;
            next.patch = 0;
        }
        VersionBump::Major => {
            next.major += 1;
            next.minor = 0;
            next.patch = 0;
        }
    }
    next.pre = Default::default();
    next.build = Default::default();
    next
}

fn workspace_member_tomls(ctx: &TaskContext) -> Result<Vec<PathBuf>> {
    let manifest = fs::read_to_string(ctx.path("Cargo.toml"))?;
    let workspace: TomlValue =
        toml::from_str(&manifest).context("failed to parse workspace Cargo.toml")?;
    let members = workspace
        .get("workspace")
        .and_then(|workspace| workspace.get("members"))
        .and_then(TomlValue::as_array)
        .ok_or_else(|| anyhow!("workspace.members missing from Cargo.toml"))?;

    let mut files = Vec::new();
    for member in members {
        let member = member
            .as_str()
            .ok_or_else(|| anyhow!("workspace member is not a string"))?;
        files.push(ctx.repo_root.join(member).join("Cargo.toml"));
    }
    Ok(files)
}

fn package_name(path: &Path) -> Result<String> {
    let manifest = fs::read_to_string(path)
        .with_context(|| format!("failed to read package manifest {}", path.display()))?;
    let document: TomlValue = toml::from_str(&manifest)
        .with_context(|| format!("failed to parse package manifest {}", path.display()))?;
    document
        .get("package")
        .and_then(|package| package.get("name"))
        .and_then(TomlValue::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow!("package.name missing from {}", path.display()))
}

fn scryer_release_member_tomls(ctx: &TaskContext) -> Result<Vec<PathBuf>> {
    workspace_member_tomls(ctx)?
        .into_iter()
        .filter_map(|path| match package_name(&path) {
            Ok(name) if !is_scryer_app_release_package(&name) => {
                println!("   excluded independent SDK crate: {name}");
                None
            }
            Ok(_) => Some(Ok(path)),
            Err(error) => Some(Err(error)),
        })
        .collect()
}

fn is_scryer_app_release_package(name: &str) -> bool {
    name != PLUGIN_SDK_PACKAGE
}

fn write_package_version(path: &Path, version: &Version) -> Result<()> {
    let mut document = fs::read_to_string(path)?
        .parse::<DocumentMut>()
        .with_context(|| format!("failed to parse {}", path.display()))?;
    document["package"]["version"] = value(version.to_string());
    fs::write(path, document.to_string())?;
    Ok(())
}

fn package_version(path: &Path) -> Result<Version> {
    let manifest = fs::read_to_string(path)
        .with_context(|| format!("failed to read package manifest {}", path.display()))?;
    let document: TomlValue = toml::from_str(&manifest)
        .with_context(|| format!("failed to parse package manifest {}", path.display()))?;
    let version = document
        .get("package")
        .and_then(|package| package.get("version"))
        .and_then(TomlValue::as_str)
        .ok_or_else(|| anyhow!("package.version missing from {}", path.display()))?;
    Version::parse(version).with_context(|| format!("invalid package.version {version}"))
}

fn parse_sdk_release_version(raw: &str) -> Result<Version> {
    let version = raw.trim();
    if version.is_empty() {
        bail!("SDK release version is required");
    }
    if version.starts_with('v') {
        bail!("pass SDK versions as plain semver, for example 1.0.0, not v1.0.0");
    }
    Version::parse(version).with_context(|| format!("invalid SDK release version {version}"))
}

fn sdk_release_tag_name(version: &Version) -> String {
    format!("{PLUGIN_SDK_TAG_PREFIX}{version}")
}

fn sdk_runtime_version_from_source(source: &str) -> Result<Version> {
    const PREFIX: &str = "pub const SDK_VERSION: &str = \"";
    for line in source.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix(PREFIX) {
            let version = rest
                .strip_suffix("\";")
                .ok_or_else(|| anyhow!("SDK_VERSION declaration is malformed"))?;
            return Version::parse(version)
                .with_context(|| format!("invalid SDK_VERSION constant {version}"));
        }
    }
    bail!("SDK_VERSION declaration missing");
}

fn sdk_runtime_version(path: &Path) -> Result<Version> {
    let source =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    sdk_runtime_version_from_source(&source)
}

fn replace_sdk_runtime_version(source: &str, version: &Version) -> Result<String> {
    const PREFIX: &str = "pub const SDK_VERSION: &str = \"";
    let mut replaced = 0usize;
    let mut output = Vec::new();
    for line in source.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with(PREFIX) {
            let indent_len = line.len() - trimmed.len();
            output.push(format!(
                "{}pub const SDK_VERSION: &str = \"{version}\";",
                &line[..indent_len]
            ));
            replaced += 1;
        } else {
            output.push(line.to_string());
        }
    }
    if replaced != 1 {
        bail!("expected exactly one SDK_VERSION declaration, found {replaced}");
    }
    let mut next = output.join("\n");
    if source.ends_with('\n') {
        next.push('\n');
    }
    Ok(next)
}

fn write_sdk_runtime_version(path: &Path, version: &Version) -> Result<()> {
    let source =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    fs::write(path, replace_sdk_runtime_version(&source, version)?)
        .with_context(|| format!("failed to write {}", path.display()))
}

fn validate_sdk_version_sync(ctx: &TaskContext, expected: &Version) -> Result<()> {
    let manifest_version = package_version(&ctx.path(PLUGIN_SDK_MANIFEST))?;
    let runtime_version = sdk_runtime_version(&ctx.path(PLUGIN_SDK_LIB))?;
    if manifest_version != *expected {
        bail!("SDK package version is {manifest_version}, expected {expected}");
    }
    if runtime_version != *expected {
        bail!("SDK_VERSION is {runtime_version}, expected {expected}");
    }
    Ok(())
}

fn status_path(line: &str) -> Option<String> {
    let path = line.get(3..)?.trim();
    let path = path.rsplit_once(" -> ").map_or(path, |(_, next)| next);
    Some(path.trim_matches('"').to_string())
}

fn sdk_release_scoped_path(path: &str) -> bool {
    path == "Cargo.lock"
        || path == ".github/workflows/plugin-sdk.yml"
        || path == "xtask/Cargo.toml"
        || path == "xtask/src/main.rs"
        || path.starts_with("crates/scryer-plugin-sdk/")
}

fn ensure_sdk_release_worktree_scope(ctx: &TaskContext) -> Result<()> {
    let status = git_capture(ctx, &["status", "--porcelain", "--untracked-files=no"])?;
    let unrelated = status
        .lines()
        .filter_map(status_path)
        .filter(|path| !sdk_release_scoped_path(path))
        .collect::<Vec<_>>();
    if !unrelated.is_empty() {
        bail!(
            "SDK release has unrelated tracked changes:\n{}",
            unrelated
                .iter()
                .map(|path| format!("  - {path}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
    Ok(())
}

struct FileSnapshot {
    path: PathBuf,
    bytes: Option<Vec<u8>>,
}

fn snapshot_files(paths: &[PathBuf]) -> Result<Vec<FileSnapshot>> {
    paths
        .iter()
        .map(|path| {
            let bytes = if path.exists() {
                Some(fs::read(path).with_context(|| format!("failed to read {}", path.display()))?)
            } else {
                None
            };
            Ok(FileSnapshot {
                path: path.clone(),
                bytes,
            })
        })
        .collect()
}

fn restore_snapshots(snapshots: Vec<FileSnapshot>) -> Result<()> {
    for snapshot in snapshots {
        match snapshot.bytes {
            Some(bytes) => fs::write(&snapshot.path, bytes)
                .with_context(|| format!("failed to restore {}", snapshot.path.display()))?,
            None if snapshot.path.exists() => fs::remove_file(&snapshot.path)
                .with_context(|| format!("failed to remove {}", snapshot.path.display()))?,
            None => {}
        }
    }
    Ok(())
}

fn changed_file(ctx: &TaskContext, path: &Path) -> Result<bool> {
    let output = git_capture(ctx, &["status", "--short", "--", &path.to_string_lossy()])?;
    Ok(!output.trim().is_empty())
}

fn add_prod_package_args(command: &mut Command) {
    for package in SCRYER_PROD_PACKAGES {
        command.args(["-p", package]);
    }
}

fn builtin_plugin_paths(ctx: &TaskContext) -> Vec<PathBuf> {
    BUILTIN_PLUGIN_ARTIFACTS
        .iter()
        .map(|path| ctx.path(path))
        .collect()
}

fn refresh_builtin_plugins(ctx: &TaskContext) -> Result<Vec<PathBuf>> {
    let plugins_dir = ctx
        .repo_root
        .parent()
        .ok_or_else(|| anyhow!("scryer repo has no parent directory"))?
        .join("scryer-plugins");
    let plugins_xtask = plugins_dir.join("xtask/Cargo.toml");
    if !plugins_xtask.is_file() {
        bail!(
            "scryer-plugins xtask not found at {}",
            plugins_xtask.display()
        );
    }

    let output_dir = ctx.path("crates/scryer-plugins/builtins");
    step("Rebuilding embedded plugin builtins");
    let mut command = repo_release_cargo_command_in(ctx, &plugins_dir)?;
    // The sibling scryer-plugins repo owns this lockfile independently, so
    // builtin refresh must not hard-fail on its local lock drift.
    command.args([
        "run",
        "--manifest-path",
        "xtask/Cargo.toml",
        "--",
        "builtins",
        "--output-dir",
    ]);
    command.arg(&output_dir);
    run_streaming(&mut command, "[plugins] ")?;
    ok("Embedded plugin builtins refreshed");

    Ok(builtin_plugin_paths(ctx))
}

fn git_checkout_paths(ctx: &TaskContext, paths: &[PathBuf]) -> Result<()> {
    if paths.is_empty() {
        return Ok(());
    }
    let mut command = ctx.release_command_in("git", &ctx.repo_root);
    command.arg("checkout").arg("--");
    command.args(paths);
    run_checked(&mut command)
}

fn run_clippy_ci(ctx: &TaskContext, args: ClippyArgs) -> Result<()> {
    let linux_target = "x86_64-unknown-linux-gnu";
    let mut rustc = ctx.command("rustc");
    rustc.arg("-vV");
    let host_target = run_capture(&mut rustc)?
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .ok_or_else(|| anyhow!("failed to determine host target"))?
        .trim()
        .to_string();
    let linux_image = std::env::var("SCRYER_LINUX_CLIPPY_IMAGE")
        .unwrap_or_else(|_| "rust:1.94-bookworm".to_string());
    let linux_platform =
        std::env::var("SCRYER_LINUX_CLIPPY_PLATFORM").unwrap_or_else(|_| "linux/arm64".to_string());

    if !args.linux_only {
        println!("Running cargo clippy for host target: {host_target}");
        let mut command = ctx.command_in("cargo", &ctx.repo_root);
        command.args(["clippy", "--workspace", "--", "-D", "warnings"]);
        run_checked(&mut command)?;
    }

    if args.linux_only || host_target != linux_target {
        if command_available("docker")? {
            println!("Running cargo clippy in Linux container: {linux_image}");
            let mut command = ctx.command("docker");
            command.args([
                "run",
                "--rm",
                "--platform",
                &linux_platform,
                "-v",
                &format!("{}:/work", ctx.repo_root.display()),
                "-w",
                "/work",
                "-e",
                "CARGO_HOME=/tmp/cargo",
                "-e",
                "CARGO_TARGET_DIR=/tmp/target",
                "-e",
                "CARGO_TERM_COLOR=always",
                &linux_image,
                "bash",
                "-lc",
                "set -euo pipefail; /usr/local/cargo/bin/rustup component add clippy; toolchain=\"$('/usr/local/cargo/bin/rustup' show active-toolchain | cut -d' ' -f1)\"; toolchain_bin=\"/usr/local/rustup/toolchains/${toolchain}/bin\"; export PATH=\"${toolchain_bin}:$PATH\"; \"${toolchain_bin}/cargo-clippy\" clippy --workspace -- -D warnings",
            ]);
            run_checked(&mut command)?;
        } else if command_available("x86_64-linux-gnu-gcc")? {
            println!("Ensuring Linux CI target is installed: {linux_target}");
            let mut target_add = ctx.command("rustup");
            target_add.args(["target", "add", linux_target]);
            run_checked(&mut target_add)?;

            println!("Running cargo clippy for Linux CI target: {linux_target}");
            let mut command = ctx.command_in("cargo", &ctx.repo_root);
            command.args([
                "clippy",
                "--workspace",
                "--target",
                linux_target,
                "--",
                "-D",
                "warnings",
            ]);
            run_checked(&mut command)?;
        } else {
            bail!("cannot run Linux CI clippy locally; install Docker or x86_64-linux-gnu-gcc");
        }
    }

    Ok(())
}

fn run_release(ctx: &TaskContext, args: ReleaseArgs) -> Result<()> {
    step("Determining next version");
    let latest_tag = latest_prefixed_tag(ctx, "scryer-v")?;
    let current_version = latest_tag
        .as_deref()
        .map(|tag| Version::parse(tag.trim_start_matches("scryer-v")))
        .transpose()?
        .unwrap_or_else(|| Version::new(0, 0, 0));
    let (bump, explicit) = parse_bump(&args)?;
    let release_args = release_args_signature(explicit.as_ref(), bump);
    let next_version = explicit.unwrap_or_else(|| next_version(&current_version, bump));
    let tag_name = format!("scryer-v{next_version}");
    let catalog_url = OFFICIAL_PLUGIN_CATALOG_URL.to_string();

    println!(
        "   Latest tag : {}",
        latest_tag.as_deref().unwrap_or("none")
    );
    println!("   Next tag   : {tag_name}");
    if args.dry_run {
        println!("   {YELLOW}(dry run — no commits, tags, or pushes){RESET}");
    }

    step("Pre-flight checks");
    let tags = git_capture(ctx, &["tag"])?;
    if tags.lines().any(|line| line == tag_name) {
        bail!("Tag {tag_name} already exists");
    }
    let branch = current_branch(ctx)?;
    let git_commit = current_head_commit(ctx)?;
    println!("   Branch : {branch}");
    let worktree_clean = git_status_porcelain(ctx)?.trim().is_empty();
    if !worktree_clean {
        prompt_continue_if_dirty(ctx)?;
    }
    require_command("gh")?;
    ok("Pre-flight OK");

    let builtin_plugin_paths = builtin_plugin_paths(ctx);
    let cache_dir = release_dry_run_cache_dir(
        ctx,
        &git_commit,
        &release_args,
        latest_tag.as_deref(),
        &next_version,
        &tag_name,
    );
    let cache_dir_relative = relative_to_repo_root(ctx, &cache_dir)?;

    let mut reused_dry_run_cache = false;
    if args.dry_run {
        clear_release_dry_run_cache(ctx)?;
        write_release_dry_run_cache(
            ctx,
            &ReleaseDryRunCache {
                success: false,
                created_at: Utc::now().to_rfc3339(),
                git_commit: git_commit.clone(),
                branch: branch.clone(),
                release_args: release_args.clone(),
                latest_tag_seen: latest_tag.clone(),
                next_version: next_version.to_string(),
                tag_name: tag_name.clone(),
                catalog_url: catalog_url.clone(),
                catalog_checksum_sha256: None,
                validated_steps: Vec::new(),
                cached_builtins_dir: Some(cache_dir_relative.clone()),
                failure_message: Some("dry run did not complete".to_string()),
            },
        )?;
    } else if worktree_clean && release_dry_run_cache_path(ctx).is_file() {
        match load_release_dry_run_cache(ctx) {
            Ok(cache) => {
                let expected_checksum = match fetch_catalog_checksum(&catalog_url) {
                    Ok(checksum) => Some(checksum),
                    Err(error) => {
                        println!(
                            "   {YELLOW}Skipping dry-run cache reuse: failed to fetch published plugin catalog checksum ({error:#}){RESET}"
                        );
                        None
                    }
                };
                if let Some(expected_checksum) = expected_checksum {
                    let next_version_text = next_version.to_string();
                    let cached_builtins_dir = cache
                        .cached_builtins_dir
                        .as_deref()
                        .map(|dir| ctx.path(dir));
                    let builtins_present = cached_builtins_dir
                        .as_ref()
                        .is_some_and(|dir| builtin_cache_complete(dir, &builtin_plugin_paths));
                    let expected = ReleaseDryRunExpectations {
                        git_commit: &git_commit,
                        release_args: &release_args,
                        latest_tag_seen: latest_tag.as_deref(),
                        next_version: &next_version_text,
                        tag_name: &tag_name,
                        catalog_checksum_sha256: &expected_checksum,
                    };
                    if let Some(reason) =
                        release_dry_run_cache_rejection_reason(&cache, &expected, builtins_present)
                    {
                        println!("   {YELLOW}Skipping dry-run cache reuse: {reason}{RESET}");
                    } else {
                        let cached_builtins_dir = cached_builtins_dir.ok_or_else(|| {
                            anyhow!("dry-run cache did not record builtin artifact directory")
                        })?;
                        step("Restoring bundled plugins from dry-run cache");
                        restore_builtin_artifacts_from_cache(
                            &cached_builtins_dir,
                            &builtin_plugin_paths,
                        )?;
                        ok("Reused dry-run cache; skipping builtin rebuild and validations");
                        reused_dry_run_cache = true;
                    }
                }
            }
            Err(error) => {
                println!("   {YELLOW}Skipping dry-run cache reuse: {error:#}{RESET}");
            }
        }
    }

    if !reused_dry_run_cache {
        let refreshed_builtin_paths = refresh_builtin_plugins(ctx)?;

        step("Running web and Rust validation in parallel");
        let (web_tx, web_rx) = mpsc::channel();
        let (rust_tx, rust_rx) = mpsc::channel();
        let web_ctx = ctx.clone();
        let rust_ctx = ctx.clone();

        thread::spawn(move || {
            let _ = web_tx.send(run_scryer_web_validation(&web_ctx, "[web] "));
        });
        thread::spawn(move || {
            let _ = rust_tx.send(run_scryer_rust_validation(&rust_ctx, "[rust] "));
        });

        let web_result = web_rx
            .recv()
            .context("web validation thread ended unexpectedly")?;
        let rust_result = rust_rx
            .recv()
            .context("rust validation thread ended unexpectedly")?;
        if let Err(error) = &web_result {
            warn(format!("Web validation failed: {error:#}"));
        }
        if let Err(error) = &rust_result {
            warn(format!("Rust validation failed: {error:#}"));
        }
        web_result?;
        rust_result?;
        ok("Parallel validation passed");

        if args.dry_run {
            let catalog_checksum_sha256 = fetch_catalog_checksum(&catalog_url)?;
            cache_builtin_artifacts(&cache_dir, &refreshed_builtin_paths)?;
            write_release_dry_run_cache(
                ctx,
                &ReleaseDryRunCache {
                    success: true,
                    created_at: Utc::now().to_rfc3339(),
                    git_commit: git_commit.clone(),
                    branch: branch.clone(),
                    release_args: release_args.clone(),
                    latest_tag_seen: latest_tag.clone(),
                    next_version: next_version.to_string(),
                    tag_name: tag_name.clone(),
                    catalog_url: catalog_url.clone(),
                    catalog_checksum_sha256: Some(catalog_checksum_sha256),
                    validated_steps: vec![
                        "builtin_refresh".to_string(),
                        "web_validation".to_string(),
                        "rust_validation".to_string(),
                    ],
                    cached_builtins_dir: Some(cache_dir_relative.clone()),
                    failure_message: None,
                },
            )?;

            let cargo_lock = ctx.path("Cargo.lock");
            let npm_lock = ctx.path("apps/scryer-web/package-lock.json");
            println!("\n{YELLOW}{BOLD}Dry run complete — stopping before commit/tag/push.{RESET}");
            println!("  Version {next_version} validated OK.");
            println!(
                "  Dry-run cache: {}",
                release_dry_run_cache_path(ctx).display()
            );
            let mut restore = refreshed_builtin_paths;
            if cargo_lock.exists() {
                restore.push(cargo_lock);
            }
            if npm_lock.exists() {
                restore.push(npm_lock);
            }
            git_checkout_paths(ctx, &restore)?;
            return Ok(());
        }
    }

    let workspace_tomls = scryer_release_member_tomls(ctx)?;
    if workspace_tomls.is_empty() {
        bail!("No workspace member Cargo.toml files found");
    }
    step(format!(
        "Updating Scryer application crate versions to {next_version}"
    ));
    for toml_path in &workspace_tomls {
        write_package_version(toml_path, &next_version)?;
        let name = toml_path
            .parent()
            .and_then(Path::file_name)
            .and_then(OsStr::to_str)
            .unwrap_or("unknown");
        println!("   bumped: {name} → {next_version}");
    }
    ok(format!(
        "{} crates updated to {}",
        workspace_tomls.len(),
        next_version
    ));

    if reused_dry_run_cache {
        ok("Skipped post-bump cargo check via dry-run cache reuse");
    } else {
        step("Running cargo check after version bump");
        let mut cargo_check = ctx.release_command_in("cargo", &ctx.repo_root);
        cargo_check.arg("check");
        add_prod_package_args(&mut cargo_check);
        run_checked(&mut cargo_check)?;
        ok("cargo check passed");
    }

    step("Committing version bump");
    let mut changed = Vec::new();
    for path in &workspace_tomls {
        if changed_file(ctx, path)? {
            changed.push(path.clone());
        }
    }
    let cargo_lock = ctx.path("Cargo.lock");
    let npm_lock = ctx.path("apps/scryer-web/package-lock.json");
    if cargo_lock.exists() && changed_file(ctx, &cargo_lock)? {
        changed.push(cargo_lock.clone());
    }
    if npm_lock.exists() && changed_file(ctx, &npm_lock)? {
        changed.push(npm_lock.clone());
    }
    for path in &builtin_plugin_paths {
        if changed_file(ctx, path)? {
            changed.push(path.clone());
        }
    }
    if !changed.is_empty() {
        let mut add = ctx.release_command_in("git", &ctx.repo_root);
        add.arg("add");
        add.args(&changed);
        run_checked(&mut add)?;
        let mut commit = ctx.release_command_in("git", &ctx.repo_root);
        commit.args([
            "commit",
            "-m",
            &format!("release: bump scryer to {next_version}"),
        ]);
        run_checked(&mut commit)?;
        ok("Committed version bump");
    } else {
        ok("Nothing to commit");
    }

    prune_scryer_release_history(ctx)?;

    step(format!("Creating signed tag {tag_name}"));
    let mut tag = ctx.release_command_in("git", &ctx.repo_root);
    tag.args(["tag", "-s", &tag_name, "-m", &format!("Release {tag_name}")]);
    run_checked(&mut tag)?;
    ok(format!("Tag {tag_name} created"));

    step("Pushing to origin");
    let mut push_branch = ctx.release_command_in("git", &ctx.repo_root);
    push_branch.args(["push", "origin", &branch]);
    run_checked(&mut push_branch)?;
    let mut push_tag = ctx.release_command_in("git", &ctx.repo_root);
    push_tag.args(["push", "origin", &tag_name]);
    run_checked(&mut push_tag)?;
    ok(format!("Pushed {branch} and tag {tag_name}"));

    println!("\n{GREEN}{BOLD}Released {tag_name}{RESET}");
    Ok(())
}

fn run_sdk_release(ctx: &TaskContext, args: SdkReleaseArgs) -> Result<()> {
    step("Preparing plugin SDK release");
    let version = parse_sdk_release_version(&args.version)?;
    let tag_name = sdk_release_tag_name(&version);
    println!("   SDK version : {version}");
    println!("   Next tag    : {tag_name}");
    if args.dry_run {
        println!("   {YELLOW}(dry run — no commits, tags, or pushes){RESET}");
    }

    step("Pre-flight checks");
    let tags = git_capture(ctx, &["tag"])?;
    if tags.lines().any(|line| line == tag_name) {
        bail!("Tag {tag_name} already exists");
    }
    let branch = current_branch(ctx)?;
    println!("   Branch : {branch}");
    ensure_sdk_release_worktree_scope(ctx)?;
    ok("SDK release scope is clean");

    let sdk_manifest = ctx.path(PLUGIN_SDK_MANIFEST);
    let sdk_lib = ctx.path(PLUGIN_SDK_LIB);
    let cargo_lock = ctx.path("Cargo.lock");
    let snapshots = snapshot_files(&[sdk_manifest.clone(), sdk_lib.clone(), cargo_lock.clone()])?;

    step("Updating SDK version metadata");
    write_package_version(&sdk_manifest, &version)?;
    write_sdk_runtime_version(&sdk_lib, &version)?;
    validate_sdk_version_sync(ctx, &version)?;
    ok("SDK package version and SDK_VERSION match");

    step("Updating Cargo.lock metadata");
    let mut cargo_check = ctx.release_command_in("cargo", &ctx.repo_root);
    cargo_check.args(["check", "-p", PLUGIN_SDK_PACKAGE]);
    run_checked(&mut cargo_check)?;
    ok("Cargo.lock metadata refreshed");

    step("Running SDK validation");
    let mut cargo_test = ctx.release_command_in("cargo", &ctx.repo_root);
    cargo_test.args(["test", "--locked", "-p", PLUGIN_SDK_PACKAGE]);
    run_checked(&mut cargo_test)?;
    let mut cargo_package = ctx.release_command_in("cargo", &ctx.repo_root);
    cargo_package.args([
        "package",
        "--locked",
        "-p",
        PLUGIN_SDK_PACKAGE,
        "--allow-dirty",
    ]);
    run_checked(&mut cargo_package)?;
    ok("SDK validation passed");

    if args.dry_run {
        restore_snapshots(snapshots)?;
        println!("\n{YELLOW}{BOLD}Dry run complete — stopping before commit/tag/push.{RESET}");
        println!("  SDK version {version} validated OK.");
        return Ok(());
    }

    step("Collecting SDK release changes");
    let status = git_capture(ctx, &["status", "--porcelain"])?;
    let mut changed = Vec::new();
    let mut unrelated = Vec::new();
    for path in status.lines().filter_map(status_path) {
        if sdk_release_scoped_path(&path) {
            changed.push(ctx.path(&path));
        } else {
            unrelated.push(path);
        }
    }
    if !unrelated.is_empty() {
        bail!(
            "SDK release produced unrelated changes:\n{}",
            unrelated
                .iter()
                .map(|path| format!("  - {path}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
    if changed.is_empty() {
        bail!("SDK release produced no changes to commit");
    }

    let mut add = ctx.release_command_in("git", &ctx.repo_root);
    add.arg("add");
    add.args(&changed);
    run_checked(&mut add)?;
    let mut commit = ctx.release_command_in("git", &ctx.repo_root);
    commit.args([
        "commit",
        "-m",
        &format!("release: publish scryer-plugin-sdk {version}"),
    ]);
    run_checked(&mut commit)?;
    ok("Committed SDK release");

    step(format!("Creating signed tag {tag_name}"));
    let mut tag = ctx.release_command_in("git", &ctx.repo_root);
    tag.args(["tag", "-s", &tag_name, "-m", &format!("Release {tag_name}")]);
    run_checked(&mut tag)?;
    ok(format!("Tag {tag_name} created"));

    step("Pushing to origin");
    let mut push_branch = ctx.release_command_in("git", &ctx.repo_root);
    push_branch.args(["push", "origin", &branch]);
    run_checked(&mut push_branch)?;
    let mut push_tag = ctx.release_command_in("git", &ctx.repo_root);
    push_tag.args(["push", "origin", &tag_name]);
    run_checked(&mut push_tag)?;
    ok(format!("Pushed {branch} and tag {tag_name}"));
    println!("\n{GREEN}{BOLD}Released {tag_name}{RESET}");
    println!("  crates.io publish will run from the plugin-sdk GitHub Actions workflow.");
    Ok(())
}

fn run_validate_trash_guides(ctx: &TaskContext) -> Result<()> {
    let release_stamp = ctx.path(".claude/trash-guides-validation-timestamp");
    require_command("claude")?;

    let smg_dir = ctx.repo_root.join("../smg");
    if !smg_dir.is_dir() {
        bail!(
            "smg repo not found at {} — required for trash guide scraper",
            smg_dir.display()
        );
    }

    step("Building trash guide scraper");
    let bin_dir = tempfile::tempdir()?;
    let bin_path = bin_dir.path().join("scrape-trash-guides");
    let mut build = ctx.release_command_in("go", &smg_dir);
    build.args(["build", "-o"]);
    build.arg(&bin_path);
    build.arg("./cmd/scrape-trash-guides");
    run_checked(&mut build)?;
    if !bin_path.is_file() {
        bail!(
            "trash guide scraper build did not produce {}",
            bin_path.display()
        );
    }
    ok("Trash guide scraper built");

    let output = NamedTempFile::new()?;
    step("Starting trash guide scraper");
    let mut command = ctx.command(&bin_path);
    command.arg("-o").arg(output.path());
    let mut scraper = command.spawn()?;

    step("Waiting for trash guide scraper");
    let status = scraper.wait()?;
    if !status.success() {
        bail!("Trash guide scraper failed");
    }
    let output_size = fs::metadata(output.path())?.len();
    if output_size == 0 {
        bail!("Trash guide scraper produced empty output");
    }
    ok(format!(
        "Trash guide scraper complete ({output_size} bytes)"
    ));

    let prompt_file = ctx.path("scripts/prompts/validate-trash-guides.md");
    step("Spawning Claude to validate release group data");
    let mut prompt = fs::read_to_string(&prompt_file)?;
    prompt.push_str("\n\n<trash-guides-json>\n");
    prompt.push_str(&fs::read_to_string(output.path())?);
    prompt.push_str("\n</trash-guides-json>\n");

    let mut claude = ctx.release_command("claude");
    claude.env("CLAUDECODE", "").arg("-p").arg(prompt).args([
        "--model",
        "claude-opus-4-6",
        "--max-turns",
        "30",
        "--allowedTools",
        "Read,Edit,Write,Glob,Grep,Bash(cargo nextest*),Bash(ls*)",
    ]);
    run_checked(&mut claude)?;
    if let Some(parent) = release_stamp.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(
        &release_stamp,
        Utc::now().format("%Y-%m-%dT%H:%M:%S%z").to_string(),
    )?;
    ok("Release group validation complete");
    Ok(())
}

fn run_scryer_web_validation(ctx: &TaskContext, prefix: &'static str) -> Result<()> {
    let web_dir = ctx.path("apps/scryer-web");
    prefixed_step(prefix, "Running npm audit fix");
    let mut audit = ctx.release_command_in("npm", &web_dir);
    audit.args(["audit", "fix"]);
    run_streaming(&mut audit, prefix)?;
    prefixed_ok(prefix, "npm audit fix complete");

    prefixed_step(prefix, "Running TypeScript type check");
    let mut lint = ctx.release_command_in("npm", &web_dir);
    lint.args(["run", "lint"]);
    run_streaming(&mut lint, prefix)?;
    prefixed_ok(prefix, "TypeScript type check passed");

    prefixed_step(prefix, "Running web build");
    let mut build = ctx.release_command_in("npm", &web_dir);
    build
        .env("SCRYER_GRAPHQL_URL", "/graphql")
        .env(
            "SCRYER_METADATA_GATEWAY_GRAPHQL_URL",
            "https://smg.scryer.media/graphql",
        )
        .args(["run", "build"]);
    run_streaming(&mut build, prefix)?;
    prefixed_ok(prefix, "Web build passed");
    Ok(())
}

fn run_scryer_rust_validation(ctx: &TaskContext, prefix: &'static str) -> Result<()> {
    prefixed_step(prefix, "Running cargo fmt --all --check");
    let mut fmt = ctx.release_command_in("cargo", &ctx.repo_root);
    fmt.args(["fmt", "--all", "--check"]);
    run_streaming(&mut fmt, prefix)?;
    prefixed_ok(prefix, "cargo fmt passed");

    prefixed_step(prefix, "Updating Cargo.lock (cargo update)");
    let mut update = ctx.release_command_in("cargo", &ctx.repo_root);
    update.arg("update");
    run_streaming(&mut update, prefix)?;
    prefixed_ok(prefix, "Cargo.lock updated");

    prefixed_step(prefix, "Running cargo audit");
    if !command_available("cargo-audit")? {
        warn("cargo-audit not installed — installing");
        let mut install = ctx.release_command_in("cargo", &ctx.repo_root);
        install.args(["install", "--locked", "cargo-audit"]);
        run_streaming(&mut install, prefix)?;
    }
    let ignores = [
        "RUSTSEC-2023-0071",
        "RUSTSEC-2026-0006",
        "RUSTSEC-2026-0020",
        "RUSTSEC-2026-0021",
        // Extism currently pins wasmtime 41.x upstream, so these remain release
        // blockers until the runtime stack moves onto a patched wasmtime line.
        "RUSTSEC-2026-0085",
        "RUSTSEC-2026-0086",
        "RUSTSEC-2026-0087",
        "RUSTSEC-2026-0088",
        "RUSTSEC-2026-0089",
        "RUSTSEC-2026-0091",
        "RUSTSEC-2026-0092",
        "RUSTSEC-2026-0093",
        "RUSTSEC-2026-0094",
        "RUSTSEC-2026-0095",
        "RUSTSEC-2026-0096",
        "RUSTSEC-2026-0114",
    ];
    warn(format!(
        "Ignoring advisories pending upstream fixes: {}",
        ignores.join(" ")
    ));
    let mut audit = ctx.release_command_in("cargo", &ctx.repo_root);
    audit.arg("audit");
    for advisory in ignores {
        audit.args(["--ignore", advisory]);
    }
    run_streaming(&mut audit, prefix)?;
    prefixed_ok(prefix, "cargo audit passed");

    prefixed_step(
        prefix,
        "Running cargo clippy for scryer production binary packages",
    );
    let mut clippy = ctx.release_command_in("cargo", &ctx.repo_root);
    clippy.arg("clippy");
    add_prod_package_args(&mut clippy);
    clippy.args(["--", "-D", "warnings"]);
    run_streaming(&mut clippy, prefix)?;
    prefixed_ok(prefix, "Clippy passed");

    prefixed_step(
        prefix,
        "Running Rust tests for scryer production binary packages",
    );
    if !command_available("cargo-nextest")? {
        warn("cargo-nextest not installed — installing");
        let mut install = ctx.release_command_in("cargo", &ctx.repo_root);
        install.args(["install", "--locked", "cargo-nextest"]);
        run_streaming(&mut install, prefix)?;
    }
    let mut nextest = ctx.release_command_in("cargo", &ctx.repo_root);
    nextest.args(["nextest", "run"]);
    add_prod_package_args(&mut nextest);
    nextest.arg("--locked");
    run_streaming(&mut nextest, prefix)?;
    prefixed_ok(prefix, "Rust tests passed");
    Ok(())
}

fn prune_scryer_release_history(ctx: &TaskContext) -> Result<()> {
    const KEEP_RELEASES: usize = 4;
    step(format!(
        "Pruning old releases and artifacts (keeping {KEEP_RELEASES} most recent)"
    ));

    let mut list = ctx.command_in("gh", &ctx.repo_root);
    list.args([
        "release",
        "list",
        "--limit",
        "100",
        "--json",
        "tagName,publishedAt",
    ]);
    let mut releases: Vec<GhRelease> = serde_json::from_str(&run_capture(&mut list)?)?;
    releases.sort_by_key(|release| release.published_at.clone());
    releases.reverse();

    let releases_to_delete = releases
        .iter()
        .skip(KEEP_RELEASES)
        .map(|release| release.tag_name.clone())
        .collect::<Vec<_>>();
    if releases_to_delete.is_empty() {
        ok("No old releases to prune");
    } else {
        for tag in &releases_to_delete {
            println!("   deleting release: {tag}");
            let mut delete = ctx.release_command_in("gh", &ctx.repo_root);
            delete.args(["release", "delete", tag, "--yes"]);
            if let Err(error) = run_checked(&mut delete) {
                warn(format!("failed to delete release {tag}: {error:#}"));
            }
        }
        ok(format!(
            "Deleted {} old release(s)",
            releases_to_delete.len()
        ));
    }

    let keep_tags = releases
        .iter()
        .take(KEEP_RELEASES)
        .map(|release| release.tag_name.clone())
        .collect::<Vec<_>>();

    let mut artifacts = ctx.command_in("gh", &ctx.repo_root);
    artifacts.args([
        "api",
        "repos/{owner}/{repo}/actions/artifacts",
        "--paginate",
        "--jq",
        ".artifacts[] | [(.id | tostring), .workflow_run.head_branch] | @tsv",
    ]);
    let artifact_rows = run_capture(&mut artifacts)?;
    let mut deleted = 0;
    for row in artifact_rows.lines() {
        let mut fields = row.split('\t');
        let Some(id) = fields.next() else {
            continue;
        };
        let Some(branch) = fields.next() else {
            continue;
        };
        if keep_tags.iter().any(|tag| tag == branch) {
            continue;
        }
        let mut delete = ctx.release_command_in("gh", &ctx.repo_root);
        delete.args([
            "api",
            "-X",
            "DELETE",
            &format!("repos/{{owner}}/{{repo}}/actions/artifacts/{id}"),
        ]);
        let _ = run_checked(&mut delete);
        deleted += 1;
    }
    if deleted == 0 {
        ok("No old artifacts to prune");
    } else {
        ok(format!("Deleted {deleted} old artifact(s)"));
    }

    let mut package_check = ctx.release_command_in("gh", &ctx.repo_root);
    package_check.args(["api", "orgs/scryer-media/packages/container/scryer"]);
    if !run_status(&mut package_check)?.success() {
        ok("No GHCR package found — skipping Docker cleanup");
        return Ok(());
    }

    let mut versions = ctx.command_in("gh", &ctx.repo_root);
    versions.args([
        "api",
        "orgs/scryer-media/packages/container/scryer/versions",
        "--paginate",
        "--jq",
        ".[] | [(.id | tostring), .created_at, ((.metadata.container.tags | length) | tostring)] | @tsv",
    ]);
    let versions = run_capture(&mut versions)?;
    let mut rows = Vec::new();
    for row in versions.lines() {
        let mut fields = row.split('\t');
        let Some(id) = fields.next() else {
            continue;
        };
        let Some(created_at) = fields.next() else {
            continue;
        };
        let Some(tag_count) = fields.next() else {
            continue;
        };
        rows.push((
            id.to_string(),
            DateTime::parse_from_rfc3339(created_at)?.with_timezone(&Utc),
            tag_count.parse::<usize>()?,
        ));
    }
    let mut tagged = rows
        .iter()
        .filter(|(_, _, tag_count)| *tag_count > 0)
        .map(|(_, created_at, _)| *created_at)
        .collect::<Vec<_>>();
    tagged.sort_by_key(|created_at| *created_at);
    tagged.reverse();
    if tagged.len() < KEEP_RELEASES {
        ok(format!(
            "Fewer than {KEEP_RELEASES} Docker releases — nothing to cull"
        ));
        return Ok(());
    }
    let cutoff = tagged[KEEP_RELEASES - 1] - Duration::seconds(60);
    let mut deleted_versions = 0;
    for (id, created_at, _) in rows {
        if created_at >= cutoff {
            continue;
        }
        let mut delete = ctx.release_command_in("gh", &ctx.repo_root);
        delete.args([
            "api",
            "--method",
            "DELETE",
            &format!("orgs/scryer-media/packages/container/scryer/versions/{id}"),
        ]);
        let _ = run_checked(&mut delete);
        deleted_versions += 1;
    }
    if deleted_versions == 0 {
        ok("No old Docker images to cull");
    } else {
        ok(format!(
            "Deleted {deleted_versions} old Docker image version(s) from ghcr.io/scryer-media/scryer"
        ));
    }
    Ok(())
}

fn compose_command(ctx: &TaskContext) -> Result<(Vec<String>, PathBuf, String)> {
    require_command("docker")?;
    let compose_file = std::env::var("SCRYER_DOCKER_COMPOSE_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| ctx.path("docker-compose.dev.yml"));
    if !compose_file.exists() {
        bail!("Compose file not found: {}", compose_file.display());
    }
    let stack_name =
        std::env::var("SCRYER_DOCKER_STACK_NAME").unwrap_or_else(|_| "scryer-dev".to_string());
    let args = vec![
        "compose".to_string(),
        "-p".to_string(),
        stack_name.clone(),
        "-f".to_string(),
        compose_file.display().to_string(),
    ];
    Ok((args, compose_file, stack_name))
}

fn docker_capture(ctx: &TaskContext, args: &[String]) -> Result<String> {
    let mut command = ctx.command("docker");
    command.args(args);
    run_capture(&mut command)
}

fn stack_restart(ctx: &TaskContext, args: StackRestartArgs) -> Result<()> {
    stack_down(ctx, StackDownArgs { all: false })?;

    let restart_services = env_list(
        "SCRYER_DOCKER_RESTART_SERVICES",
        &["scryer", "nodejs", "proxy"],
    );
    if restart_services.iter().any(|service| service == "scryer") {
        reset_scryer_config_volume(ctx)?;
    }

    stack_up(ctx, StackUpArgs { seed: args.seed })
}

fn reset_scryer_config_volume(ctx: &TaskContext) -> Result<()> {
    let (_, _, stack_name) = compose_command(ctx)?;
    let volume_name = format!("{stack_name}-scryer-config");

    let mut inspect = ctx.command("docker");
    inspect.args(["volume", "inspect", &volume_name]);
    if !inspect.output()?.status.success() {
        ok(format!("Scryer config volume {volume_name} already absent"));
        return Ok(());
    }

    step(format!("Resetting scryer config volume ({volume_name})"));
    let mut remove = ctx.command("docker");
    remove.args(["volume", "rm", "-f", &volume_name]);
    run_checked(&mut remove)?;
    ok(format!("Removed {volume_name}"));
    Ok(())
}

fn stack_up(ctx: &TaskContext, args: StackUpArgs) -> Result<()> {
    let (compose_base, _, _) = compose_command(ctx)?;
    // SAFETY: xtask is a single-process CLI and the env var only needs to apply
    // to child docker-compose invocations during this command.
    unsafe {
        std::env::set_var("SCRYER_AUTH_ENABLED", "false");
    }
    for path in [
        "tmp/scryer-data",
        "tmp/scryer-media/movies",
        "tmp/scryer-media/series",
        "tmp/nzbget/config",
        "tmp/nzbget-downloads",
        "tmp/weaver/data",
        "tmp/weaver-downloads",
    ] {
        fs::create_dir_all(ctx.path(path))?;
    }
    let restart_services = env_list(
        "SCRYER_DOCKER_RESTART_SERVICES",
        &["scryer", "nodejs", "proxy"],
    );
    let infra_services = env_list("SCRYER_DOCKER_INFRA_SERVICES", &["nzbget", "weaver"]);
    let force_infra_restart = std::env::var("SCRYER_DOCKER_FORCE_INFRA_RESTART")
        .map(|value| value == "1")
        .unwrap_or(false);

    let mut rm = ctx.command("docker");
    rm.args(&compose_base).args(["rm", "-sf", "scryer"]);
    let _ = run_status(&mut rm);

    if force_infra_restart {
        compose_up(ctx, &compose_base, false, &infra_services)?;
    } else {
        let mut ps_args = compose_base.clone();
        ps_args.extend([
            "ps".to_string(),
            "--services".to_string(),
            "--filter".to_string(),
            "status=running".to_string(),
        ]);
        let running = docker_capture(ctx, &ps_args)?;
        let running = running.lines().map(ToOwned::to_owned).collect::<Vec<_>>();
        let to_start = infra_services
            .iter()
            .filter(|service| !running.iter().any(|running| running == *service))
            .cloned()
            .collect::<Vec<_>>();
        if !to_start.is_empty() {
            compose_up(ctx, &compose_base, false, &to_start)?;
        }
    }

    let mut proxy_requested = false;
    let mut non_proxy = Vec::new();
    for service in &restart_services {
        if service == "proxy" {
            proxy_requested = true;
        } else {
            non_proxy.push(service.clone());
        }
    }
    compose_up(ctx, &compose_base, true, &non_proxy)?;

    if restart_services.iter().any(|service| service == "scryer") || proxy_requested {
        wait_for_scryer()?;
    }
    if restart_services.iter().any(|service| service == "nodejs") || proxy_requested {
        wait_for_nodejs()?;
    }
    if args.seed {
        ensure_seed_xtask_binary(ctx)?;
        compose_up(ctx, &compose_base, false, &["seed".to_string()])?;
    }
    if proxy_requested {
        compose_up(ctx, &compose_base, true, &["proxy".to_string()])?;
    }
    Ok(())
}

fn ensure_seed_xtask_binary(ctx: &TaskContext) -> Result<()> {
    let binary = ctx.path("tmp/xtask-seed-target/release/xtask");
    if binary.is_file() {
        return Ok(());
    }

    step("Building Linux xtask binary for the seed container");
    let mut docker_build = ctx.command("docker");
    docker_build.args([
        "build",
        "-q",
        "-f",
        "docker/scryer-dev-runtime.Dockerfile",
        ".",
    ]);
    let image = run_capture(&mut docker_build)?.trim().to_string();
    if image.is_empty() {
        bail!("failed to resolve dev runtime image id for seed binary build");
    }

    let target_dir = ctx.path("tmp/xtask-seed-target");
    fs::create_dir_all(&target_dir)?;
    let uid = capture_command_text(ctx, "id", &["-u"]).unwrap_or_else(|_| "0".to_string());
    let gid = capture_command_text(ctx, "id", &["-g"]).unwrap_or_else(|_| "0".to_string());
    let mut docker_run = ctx.command("docker");
    docker_run.args([
        "run",
        "--rm",
        "--user",
        &format!("{uid}:{gid}"),
        "-v",
        &format!("{}:/workspace", ctx.repo_root.display()),
        "-w",
        "/workspace",
        &image,
        "cargo",
        "build",
        "-p",
        "xtask",
        "--release",
        "--target-dir",
        "/workspace/tmp/xtask-seed-target",
    ]);
    run_checked(&mut docker_run)?;

    if !binary.is_file() {
        bail!(
            "seed xtask binary missing after build: {}",
            binary.display()
        );
    }
    ok(format!("Seed binary ready at {}", binary.display()));
    Ok(())
}

fn capture_command_text(ctx: &TaskContext, program: &str, args: &[&str]) -> Result<String> {
    let mut command = ctx.command(program);
    command.args(args);
    Ok(run_capture(&mut command)?.trim().to_string())
}

fn compose_up(
    ctx: &TaskContext,
    compose_base: &[String],
    no_deps: bool,
    services: &[String],
) -> Result<()> {
    if services.is_empty() {
        return Ok(());
    }
    let mut command = ctx.command("docker");
    command
        .args(compose_base)
        .args(["up", "-d", "--remove-orphans"]);
    if no_deps {
        command.arg("--no-deps");
    }
    command.args(services);
    run_checked(&mut command)
}

fn env_list(name: &str, defaults: &[&str]) -> Vec<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.split_whitespace().map(ToOwned::to_owned).collect())
        .unwrap_or_else(|| defaults.iter().map(|value| value.to_string()).collect())
}

fn wait_for_scryer() -> Result<()> {
    println!("Waiting for scryer to be ready...");
    let timeout = std::env::var("SCRYER_DOCKER_SCRYER_READY_TIMEOUT_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(600);
    let poll = std::env::var("SCRYER_DOCKER_READY_POLL_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(2);
    let attempts = std::cmp::max(1, timeout / poll);
    for _ in 0..attempts {
        let status = Command::new("curl")
            .args(["-sf", "http://localhost:8080/health"])
            .status()?;
        if status.success() {
            return Ok(());
        }
        if matches!(
            docker_inspect_state("scryer")?.as_deref(),
            Some("exited" | "dead")
        ) {
            warn("scryer exited before it became ready");
            log_container_failure("scryer")?;
            bail!("scryer exited before it became ready");
        }
        thread::sleep(std::time::Duration::from_secs(poll));
    }
    warn("Timed out waiting for scryer to become ready");
    log_container_failure("scryer")?;
    bail!("Timed out waiting for scryer to become ready");
}

fn wait_for_nodejs() -> Result<()> {
    println!("Waiting for nodejs to be ready...");
    let timeout = std::env::var("SCRYER_DOCKER_NODEJS_READY_TIMEOUT_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(120);
    let poll = std::env::var("SCRYER_DOCKER_READY_POLL_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(2);
    let attempts = std::cmp::max(1, timeout / poll);
    for _ in 0..attempts {
        match docker_inspect_state("scryer-nodejs")?.as_deref() {
            Some("running") => {
                let status = Command::new("docker")
                    .args([
                        "exec",
                        "scryer-nodejs",
                        "sh",
                        "-lc",
                        "wget -q -O /dev/null http://127.0.0.1:3000",
                    ])
                    .status()?;
                if status.success() {
                    return Ok(());
                }
            }
            Some("exited" | "dead") => {
                warn("nodejs exited before it became ready");
                log_container_failure("scryer-nodejs")?;
                bail!("nodejs exited before it became ready");
            }
            _ => {}
        }
        thread::sleep(std::time::Duration::from_secs(poll));
    }
    warn("Timed out waiting for nodejs to become ready");
    log_container_failure("scryer-nodejs")?;
    bail!("Timed out waiting for nodejs to become ready");
}

fn docker_inspect_state(container: &str) -> Result<Option<String>> {
    let output = Command::new("docker")
        .args(["inspect", "--format", "{{.State.Status}}", container])
        .output()?;
    if !output.status.success() {
        return Ok(None);
    }
    Ok(Some(
        String::from_utf8_lossy(&output.stdout).trim().to_string(),
    ))
}

fn log_container_failure(container: &str) -> Result<()> {
    eprintln!("Recent logs for {container}:");
    let mut command = Command::new("docker");
    command.args(["logs", "--tail", "200", container]);
    let _ = run_status(&mut command);
    Ok(())
}

fn stack_down(ctx: &TaskContext, args: StackDownArgs) -> Result<()> {
    let (compose_base, _, _) = compose_command(ctx)?;
    let restart_services = env_list(
        "SCRYER_DOCKER_RESTART_SERVICES",
        &["scryer", "nodejs", "proxy"],
    );
    let infra_services = env_list("SCRYER_DOCKER_INFRA_SERVICES", &["nzbget"]);
    let stop_infra = std::env::var("SCRYER_DOCKER_STOP_INFRA")
        .map(|value| value == "1")
        .unwrap_or(false);

    if args.all {
        let mut command = ctx.command("docker");
        command
            .args(&compose_base)
            .args(["down", "--remove-orphans"]);
        return run_checked(&mut command);
    }

    let mut services = restart_services;
    if stop_infra {
        services.extend(infra_services);
    }

    let mut stop = ctx.command("docker");
    stop.args(&compose_base).arg("stop").args(&services);
    run_checked(&mut stop)?;

    let mut rm = ctx.command("docker");
    rm.args(&compose_base).args(["rm", "-f"]).args(&services);
    run_checked(&mut rm)?;
    Ok(())
}

fn stack_logs(ctx: &TaskContext, args: StackLogsArgs) -> Result<()> {
    let (compose_base, _, _) = compose_command(ctx)?;
    let service = args.service.unwrap_or_else(|| {
        std::env::var("SCRYER_DOCKER_LOG_SERVICE").unwrap_or_else(|_| "scryer".to_string())
    });
    let lines = std::env::var("SCRYER_STACK_LINES").unwrap_or_else(|_| "200".to_string());
    if lines.parse::<u64>().is_err() {
        bail!("SCRYER_STACK_LINES must be numeric.");
    }
    let mut command = ctx.command("docker");
    command
        .args(&compose_base)
        .args(["logs", &service, "-n", &lines, "-f"]);
    run_checked(&mut command)
}

fn nzbget_up(ctx: &TaskContext) -> Result<()> {
    let repo_dir = std::env::var("SCRYER_REPO_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| ctx.repo_root.clone());
    let nzbget_bin =
        std::env::var("NZBGET_BIN").unwrap_or_else(|_| "/opt/homebrew/bin/nzbget".to_string());
    let web_dir = std::env::var("NZBGET_WEB_DIR")
        .unwrap_or_else(|_| "/opt/homebrew/opt/nzbget/share/nzbget/webui".to_string());
    let config_template = std::env::var("NZBGET_CONFIG_TEMPLATE")
        .unwrap_or_else(|_| "/opt/homebrew/opt/nzbget/share/nzbget/nzbget.conf".to_string());
    let cert_store = std::env::var("NZBGET_CERT_STORE").unwrap_or_else(|_| {
        "/opt/homebrew/opt/ca-certificates/share/ca-certificates/cacert.pem".to_string()
    });
    let conf = std::env::var("NZBGET_CONF")
        .map(PathBuf::from)
        .unwrap_or_else(|_| repo_dir.join("tmp/nzbget/config/nzbget.conf"));
    let download_dir = std::env::var("NZBGET_DOWNLOAD_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| repo_dir.join("tmp/nzbget/downloads"));

    if !Path::new(&nzbget_bin).exists() && !command_available(&nzbget_bin)? {
        bail!("NZBGet binary not found or not executable: {nzbget_bin}");
    }
    if !conf.exists() {
        bail!(
            "NZBGet config file not found at {}. Copy your nzbget.conf there before starting.",
            conf.display()
        );
    }
    fs::create_dir_all(download_dir)?;
    println!("Starting NZBGet with {}", conf.display());

    let mut command = ctx.command(&nzbget_bin);
    command.args([
        "-D",
        "-o",
        "OutputMode=loggable",
        "-o",
        &format!("WebDir={web_dir}"),
        "-o",
        &format!("ConfigTemplate={config_template}"),
    ]);
    if !cert_store.is_empty() {
        command.args(["-o", &format!("CertStore={cert_store}")]);
    }
    command.args(["-c", &conf.display().to_string()]);
    run_checked(&mut command)
}

fn nzbget_down(ctx: &TaskContext) -> Result<()> {
    let repo_dir = std::env::var("SCRYER_REPO_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| ctx.repo_root.clone());
    let nzbget_bin =
        std::env::var("NZBGET_BIN").unwrap_or_else(|_| "/opt/homebrew/bin/nzbget".to_string());
    let conf = std::env::var("NZBGET_CONF")
        .map(PathBuf::from)
        .unwrap_or_else(|_| repo_dir.join("tmp/nzbget/config/nzbget.conf"));
    let pattern = format!("{nzbget_bin} .* -c {}", conf.display());

    let mut pgrep = ctx.command("pgrep");
    pgrep.args(["-f", &pattern]);
    if run_status(&mut pgrep)?.success() {
        let mut pkill = ctx.command("pkill");
        pkill.args(["-f", &pattern]);
        run_checked(&mut pkill)?;
        println!("NZBGet stopped.");
    } else {
        println!("NZBGet is not running.");
    }
    Ok(())
}

fn profile_hotpaths(ctx: &TaskContext, args: ProfileHotpathsArgs) -> Result<()> {
    profile::run(ctx, args)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_release_dry_run_cache() -> ReleaseDryRunCache {
        ReleaseDryRunCache {
            success: true,
            created_at: "2026-05-02T00:00:00Z".to_string(),
            git_commit: "abc123".to_string(),
            branch: "main".to_string(),
            release_args: "bump:patch".to_string(),
            latest_tag_seen: Some("scryer-v0.13.1".to_string()),
            next_version: "0.13.2".to_string(),
            tag_name: "scryer-v0.13.2".to_string(),
            catalog_url: OFFICIAL_PLUGIN_CATALOG_URL.to_string(),
            catalog_checksum_sha256: Some("deadbeef".to_string()),
            validated_steps: vec!["builtin_refresh".to_string()],
            cached_builtins_dir: Some("tmp/cache".to_string()),
            failure_message: None,
        }
    }

    fn sample_release_dry_run_expectations<'a>() -> ReleaseDryRunExpectations<'a> {
        ReleaseDryRunExpectations {
            git_commit: "abc123",
            release_args: "bump:patch",
            latest_tag_seen: Some("scryer-v0.13.1"),
            next_version: "0.13.2",
            tag_name: "scryer-v0.13.2",
            catalog_checksum_sha256: "deadbeef",
        }
    }

    #[test]
    fn sdk_release_tag_uses_independent_prefix() {
        let version = Version::parse("1.0.0").unwrap();
        assert_eq!(sdk_release_tag_name(&version), "plugin-sdk-v1.0.0");
    }

    #[test]
    fn sdk_release_version_rejects_leading_v() {
        assert!(parse_sdk_release_version("v1.0.0").is_err());
    }

    #[test]
    fn sdk_release_scope_excludes_unrelated_paths() {
        assert!(sdk_release_scoped_path(
            "crates/scryer-plugin-sdk/src/lib.rs"
        ));
        assert!(sdk_release_scoped_path("Cargo.lock"));
        assert!(sdk_release_scoped_path(".github/workflows/plugin-sdk.yml"));
        assert!(sdk_release_scoped_path("xtask/Cargo.toml"));
        assert!(sdk_release_scoped_path("xtask/src/main.rs"));
        assert!(!sdk_release_scoped_path(
            "crates/scryer-application/Cargo.toml"
        ));
    }

    #[test]
    fn sdk_runtime_version_round_trips_constant() {
        let source = "pub const SDK_VERSION: &str = \"1.4.0\";\n";
        let updated =
            replace_sdk_runtime_version(source, &Version::parse("1.0.0").unwrap()).unwrap();
        assert_eq!(
            sdk_runtime_version_from_source(&updated).unwrap(),
            Version::parse("1.0.0").unwrap()
        );
    }

    #[test]
    fn app_release_package_filter_excludes_plugin_sdk() {
        assert!(!is_scryer_app_release_package("scryer-plugin-sdk"));
        assert!(is_scryer_app_release_package("scryer"));
    }

    #[test]
    fn release_args_signature_uses_bump_mode_when_version_not_explicit() {
        assert_eq!(
            release_args_signature(None, VersionBump::Minor),
            "bump:minor"
        );
    }

    #[test]
    fn release_args_signature_uses_explicit_version_when_present() {
        let version = Version::parse("1.2.3").unwrap();
        assert_eq!(
            release_args_signature(Some(&version), VersionBump::Patch),
            "version:1.2.3"
        );
    }

    #[test]
    fn release_dry_run_cache_round_trips_through_json() {
        let cache = sample_release_dry_run_cache();
        let json = serde_json::to_string(&cache).unwrap();
        let decoded: ReleaseDryRunCache = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, cache);
    }

    #[test]
    fn release_dry_run_cache_rejects_unsuccessful_prior_run() {
        let mut cache = sample_release_dry_run_cache();
        cache.success = false;
        let reason = release_dry_run_cache_rejection_reason(
            &cache,
            &sample_release_dry_run_expectations(),
            true,
        );
        assert_eq!(
            reason.as_deref(),
            Some("previous dry run did not complete successfully")
        );
    }

    #[test]
    fn release_dry_run_cache_rejects_commit_mismatch() {
        let mut cache = sample_release_dry_run_cache();
        cache.git_commit = "def456".to_string();
        let reason = release_dry_run_cache_rejection_reason(
            &cache,
            &sample_release_dry_run_expectations(),
            true,
        );
        assert_eq!(reason.as_deref(), Some("HEAD commit changed since dry run"));
    }

    #[test]
    fn release_dry_run_cache_rejects_args_mismatch() {
        let mut cache = sample_release_dry_run_cache();
        cache.release_args = "bump:minor".to_string();
        let reason = release_dry_run_cache_rejection_reason(
            &cache,
            &sample_release_dry_run_expectations(),
            true,
        );
        assert_eq!(
            reason.as_deref(),
            Some("release arguments changed since dry run")
        );
    }

    #[test]
    fn release_dry_run_cache_rejects_latest_tag_mismatch() {
        let mut cache = sample_release_dry_run_cache();
        cache.latest_tag_seen = Some("scryer-v0.13.0".to_string());
        let reason = release_dry_run_cache_rejection_reason(
            &cache,
            &sample_release_dry_run_expectations(),
            true,
        );
        assert_eq!(
            reason.as_deref(),
            Some("latest release tag changed since dry run")
        );
    }

    #[test]
    fn release_dry_run_cache_rejects_next_tag_mismatch() {
        let mut cache = sample_release_dry_run_cache();
        cache.tag_name = "scryer-v0.13.3".to_string();
        let reason = release_dry_run_cache_rejection_reason(
            &cache,
            &sample_release_dry_run_expectations(),
            true,
        );
        assert_eq!(
            reason.as_deref(),
            Some("computed release tag changed since dry run")
        );
    }

    #[test]
    fn release_dry_run_cache_rejects_catalog_checksum_mismatch() {
        let mut cache = sample_release_dry_run_cache();
        cache.catalog_checksum_sha256 = Some("cafebabe".to_string());
        let reason = release_dry_run_cache_rejection_reason(
            &cache,
            &sample_release_dry_run_expectations(),
            true,
        );
        assert_eq!(
            reason.as_deref(),
            Some("published plugin catalog checksum changed since dry run")
        );
    }

    #[test]
    fn release_dry_run_cache_rejects_missing_cached_builtins() {
        let cache = sample_release_dry_run_cache();
        let reason = release_dry_run_cache_rejection_reason(
            &cache,
            &sample_release_dry_run_expectations(),
            false,
        );
        assert_eq!(
            reason.as_deref(),
            Some("cached builtin artifacts are missing")
        );
    }

    #[test]
    fn release_dry_run_cache_accepts_matching_inputs() {
        let cache = sample_release_dry_run_cache();
        let reason = release_dry_run_cache_rejection_reason(
            &cache,
            &sample_release_dry_run_expectations(),
            true,
        );
        assert!(reason.is_none());
    }
}
