use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use chrono::{DateTime, Duration, Utc};
use clap::{Args, Parser, Subcommand, ValueEnum};
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sigstore::{
    cosign::{CosignCapabilities, bundle::SignedArtifactBundle},
    crypto::{CosignVerificationKey, SigningScheme},
    trust::{TrustRoot, sigstore::SigstoreTrustRoot},
};
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{ChildStderr, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::{Arc, OnceLock, mpsc};
use std::thread;
use tempfile::NamedTempFile;
use toml::Value as TomlValue;
use toml_edit::{DocumentMut, value};
use x509_cert::{
    Certificate,
    der::DecodePem,
    ext::{
        Extension,
        pkix::{SubjectAltName, name::GeneralName},
    },
};

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
const RELEASE_DRY_RUN_CACHE_FILE: &str = "tmp/xtask-release-dry-run.json";
const RELEASE_DRY_RUN_BUILTINS_DIR: &str = "tmp/xtask-release-dry-run-builtins";
const OFFICIAL_PLUGIN_CATALOG_URL: &str =
    "https://github.com/scryer-media/scryer-plugins/releases/download/catalog%2Fv2/catalog-v2.json";
const BUILTIN_ASSET_DIR: &str = "crates/scryer-plugins/builtins";
const OFFICIAL_PLUGIN_REPO: &str = "scryer-media/scryer-plugins";
const OFFICIAL_RELEASE_WORKFLOW: &str = ".github/workflows/release-plugin.yml";
const SIGSTORE_GITHUB_WORKFLOW_NAME_OID: &str = "1.3.6.1.4.1.57264.1.4";
const SIGSTORE_GITHUB_WORKFLOW_REPOSITORY_OID: &str = "1.3.6.1.4.1.57264.1.5";
const SIGSTORE_GITHUB_WORKFLOW_REF_OID: &str = "1.3.6.1.4.1.57264.1.6";

type RekorVerificationKeys = BTreeMap<String, CosignVerificationKey>;

static REKOR_VERIFICATION_KEYS: OnceLock<Result<Arc<RekorVerificationKeys>, String>> =
    OnceLock::new();

struct BuiltinPluginSpec {
    plugin_id: &'static str,
    artifact_stem: &'static str,
}

#[derive(Clone, Debug, Deserialize)]
struct CatalogV2 {
    plugins: Vec<CatalogV2Entry>,
}

#[derive(Clone, Debug, Deserialize)]
struct CatalogV2Entry {
    id: String,
    child_catalog_url: String,
    required_signer: RequiredSignerV2,
}

#[derive(Clone, Debug, Deserialize)]
struct RequiredSignerV2 {
    github_repository: String,
    #[serde(default)]
    github_workflow: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct ChildCatalogV2 {
    id: String,
    description: String,
    releases: Vec<ChildCatalogReleaseV2>,
}

#[derive(Clone, Debug, Deserialize)]
struct ChildCatalogReleaseV2 {
    version: String,
    artifact_manifest_url: String,
}

#[derive(Clone, Debug, Deserialize)]
struct PluginManifestV2 {
    id: String,
    version: String,
    artifact: String,
    compression: String,
    wasm_digest: String,
    artifact_digest: String,
    signature: String,
}

const BUILTIN_PLUGINS: &[BuiltinPluginSpec] = &[
    BuiltinPluginSpec {
        plugin_id: "animetosho",
        artifact_stem: "animetosho_indexer",
    },
    BuiltinPluginSpec {
        plugin_id: "newznab",
        artifact_stem: "newznab_indexer",
    },
    BuiltinPluginSpec {
        plugin_id: "nzbgeek",
        artifact_stem: "nzbgeek_indexer",
    },
    BuiltinPluginSpec {
        plugin_id: "torznab",
        artifact_stem: "torznab_indexer",
    },
];

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
    Builtins(BuiltinsArgs),
    Sdk(SdkArgs),
    ValidateTrashGuides,
    Ci(CiArgs),
    Stack(StackArgs),
    Serve(ServeArgs),
    Nzbget(NzbgetArgs),
    Seed(SeedArgs),
    Profile(ProfileArgs),
    Corpus(CorpusArgs),
}

#[derive(Args)]
struct BuiltinsArgs {
    #[command(subcommand)]
    command: BuiltinsCommand,
}

#[derive(Subcommand)]
enum BuiltinsCommand {
    Sync,
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
struct ServeArgs {
    #[arg(
        long,
        default_value = "127.0.0.1:18080",
        help = "Bind address for the locally hosted Scryer debug server"
    )]
    bind: String,
    #[arg(
        long,
        default_value_t = 3000,
        help = "Port for the Vite dev server with hot reload"
    )]
    frontend_port: u16,
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
    worktree_clean_at_start: bool,
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
        Commands::Builtins(args) => match args.command {
            BuiltinsCommand::Sync => {
                refresh_builtin_plugins(&ctx)?;
                Ok(())
            }
        },
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
        Commands::Serve(args) => serve_local_scryer(&ctx, args),
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

fn tail_file(path: &Path, lines: usize) -> Result<String> {
    let content = fs::read_to_string(path).unwrap_or_default();
    let collected = content.lines().rev().take(lines).collect::<Vec<_>>();
    Ok(collected.into_iter().rev().collect::<Vec<_>>().join("\n"))
}

fn wait_for_local_backend(backend: &mut std::process::Child, port: u16, log_path: &Path) -> Result<()> {
    let address = format!("127.0.0.1:{port}");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
    while std::time::Instant::now() < deadline {
        if let Some(status) = backend.try_wait()? {
            let tail = tail_file(log_path, 50)?;
            bail!(
                "Scryer failed to start on http://{address}/ (status: {status}). Tail of {}:\n{tail}",
                log_path.display()
            );
        }

        if std::net::TcpStream::connect(&address).is_ok() {
            return Ok(());
        }

        thread::sleep(std::time::Duration::from_millis(250));
    }

    let tail = tail_file(log_path, 50)?;
    bail!(
        "Timed out waiting for Scryer on http://{address}/. Tail of {}:\n{tail}",
        log_path.display()
    )
}

fn backend_port(bind: &str) -> Result<u16> {
    let (_, port) = bind
        .rsplit_once(':')
        .ok_or_else(|| anyhow!("bind address must include a port: {bind}"))?;
    port.parse::<u16>()
        .with_context(|| format!("invalid port in bind address: {bind}"))
}

fn resolve_frontend_port(preferred: u16) -> Result<u16> {
    for offset in 0..=20u16 {
        let Some(candidate) = preferred.checked_add(offset) else {
            break;
        };
        if std::net::TcpListener::bind((std::net::Ipv4Addr::UNSPECIFIED, candidate)).is_ok() {
            return Ok(candidate);
        }
    }

    bail!("could not find an open Vite dev-server port starting at {preferred}")
}

fn serve_db_path() -> Result<PathBuf> {
    #[cfg(target_os = "macos")]
    let base_dir = std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join("Library/Application Support/scryer"))
        .unwrap_or_else(|| PathBuf::from("./scryer"));

    #[cfg(target_os = "linux")]
    let base_dir = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(PathBuf::from).map(|home| home.join(".local/share"))
        })
        .map(|base| base.join("scryer"))
        .unwrap_or_else(|| PathBuf::from("./scryer"));

    #[cfg(target_os = "windows")]
    let base_dir = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .map(|base| base.join("scryer"))
        .unwrap_or_else(|| PathBuf::from("./scryer"));

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    let base_dir = PathBuf::from("./scryer");

    let db_dir = base_dir.join("xtask");
    fs::create_dir_all(&db_dir)?;
    Ok(db_dir.join("scryer.db"))
}

fn serve_encryption_key() -> String {
    let digest = Sha256::digest(b"scryer-xtask-dev-encryption-key");
    base64::engine::general_purpose::STANDARD.encode(digest)
}

fn serve_local_scryer(ctx: &TaskContext, args: ServeArgs) -> Result<()> {
    require_command("npm")?;

    let env_file = ctx.path(".env");
    let mut dotenv_envs = Vec::new();
    if env_file.is_file() {
        let dotenv_iter = dotenvy::from_path_iter(&env_file)
            .with_context(|| format!("failed to read {}", env_file.display()))?;
        for entry in dotenv_iter {
            let (key, value) =
                entry.with_context(|| format!("failed to parse {}", env_file.display()))?;
            dotenv_envs.push((key, value));
        }
    }

    let web_dir = ctx.path("apps/scryer-web");
    let backend_port = backend_port(&args.bind)?;
    let frontend_port = resolve_frontend_port(args.frontend_port)?;
    let backend_url = format!("http://127.0.0.1:{backend_port}");
    let frontend_url = format!("http://127.0.0.1:{frontend_port}");
    let db_path = serve_db_path()?;
    let db_url = format!("sqlite://{}", db_path.display());
    let encryption_key = serve_encryption_key();
    let backend_log = PathBuf::from(
        std::env::var("SCRYER_DEV_BACKEND_LOG")
            .unwrap_or_else(|_| "/tmp/scryer-dev-backend.log".to_string()),
    );
    if let Some(parent) = backend_log.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&backend_log, "")?;

    step(format!("Starting Scryer backend with cargo run on {}", args.bind));
    if env_file.is_file() {
        ok(format!("Loaded runtime environment from {}", env_file.display()));
    }
    if frontend_port != args.frontend_port {
        warn(format!(
            "frontend port {} is busy; using {} for the Vite dev server",
            args.frontend_port, frontend_port
        ));
    }
    println!("   Vite dev server: {frontend_url}");
    println!("   Keychain: disabled for xtask serve");

    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&backend_log)?;
    let log_err = log.try_clone()?;
    let mut serve = ctx.command_in("cargo", &ctx.repo_root);
    for (key, value) in &dotenv_envs {
        serve.env(key, value);
    }
    serve
        .env("SCRYER_DB_PATH", &db_url)
        .env("SCRYER_DISABLE_PLATFORM_KEYSTORE", "1")
        .env("SCRYER_ENCRYPTION_KEY", &encryption_key)
        .env("SCRYER_OPEN_BROWSER", "false")
        .env("SCRYER_WEB_UI_URL", &frontend_url)
        .env("SCRYER_BIND", &args.bind)
        .args(["run", "--locked", "-p", "scryer"])
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log_err));
    let mut backend = serve.spawn()?;
    if let Err(error) = wait_for_local_backend(&mut backend, backend_port, &backend_log) {
        let _ = backend.kill();
        let _ = backend.wait();
        return Err(error);
    }

    println!("==> Scryer backend ready");
    println!("    Backend:  {backend_url}");
    println!("    Frontend: {frontend_url}");
    println!("    Database: {}", db_path.display());
    println!("    Log:      tail -f {}", backend_log.display());
    println!();
    println!("==> Starting Vite dev server with live updates...");

    let mut vite = ctx.command_in("npm", &web_dir);
    vite.env("SCRYER_DEV_PROXY_TARGET", &backend_url).args([
        "run",
        "dev",
        "--",
        "--host",
        "0.0.0.0",
        "--strictPort",
        "--port",
        &frontend_port.to_string(),
    ]);
    let result = run_status(&mut vite);

    let _ = backend.kill();
    let _ = backend.wait();
    result?;
    Ok(())
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

fn git_tracked_dirty_paths(ctx: &TaskContext) -> Result<Vec<PathBuf>> {
    let mut command = ctx.command_in("git", &ctx.repo_root);
    command.args(["diff", "--name-only", "HEAD", "--"]);
    let output = run_capture(&mut command)?;
    Ok(output
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| ctx.path(line))
        .collect())
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

fn commit_tracked_changes(
    ctx: &TaskContext,
    paths: &[PathBuf],
    message: &str,
) -> Result<Option<String>> {
    if paths.is_empty() {
        return Ok(None);
    }

    let mut add = ctx.release_command_in("git", &ctx.repo_root);
    add.arg("add");
    add.args(paths);
    run_checked(&mut add)?;

    let mut commit = ctx.release_command_in("git", &ctx.repo_root);
    commit.args(["commit", "-m", message]);
    run_checked(&mut commit)?;

    Ok(Some(current_head_commit(ctx)?))
}

fn add_prod_package_args(command: &mut Command) {
    for package in SCRYER_PROD_PACKAGES {
        command.args(["-p", package]);
    }
}

struct BuiltinAssetPaths {
    wasm: PathBuf,
    descriptor_json: PathBuf,
    description: PathBuf,
}

fn builtin_asset_paths(ctx: &TaskContext, spec: &BuiltinPluginSpec) -> BuiltinAssetPaths {
    let dir = ctx.path(BUILTIN_ASSET_DIR);
    BuiltinAssetPaths {
        wasm: dir.join(format!("{}.wasm.zst", spec.artifact_stem)),
        descriptor_json: dir.join(format!("{}.descriptor.json", spec.artifact_stem)),
        description: dir.join(format!("{}.description.txt", spec.artifact_stem)),
    }
}

fn builtin_plugin_paths(ctx: &TaskContext) -> Vec<PathBuf> {
    BUILTIN_PLUGINS
        .iter()
        .flat_map(|spec| {
            let paths = builtin_asset_paths(ctx, spec);
            [paths.wasm, paths.descriptor_json, paths.description]
        })
        .collect()
}

fn bundle_url_for(url: &str) -> String {
    format!("{url}.bundle")
}

fn fetch_url_bytes(url: &str) -> Result<Vec<u8>> {
    let response = reqwest::blocking::get(url)
        .with_context(|| format!("failed to fetch {url}"))?
        .error_for_status()
        .with_context(|| format!("request returned error status for {url}"))?;
    Ok(response
        .bytes()
        .with_context(|| format!("failed to read response body for {url}"))?
        .to_vec())
}

fn decode_possibly_zstd_bytes(url: &str, bytes: Vec<u8>) -> Result<Vec<u8>> {
    if url.ends_with(".zst") {
        return zstd::decode_all(bytes.as_slice())
            .with_context(|| format!("failed to decompress {url}"));
    }
    Ok(bytes)
}

fn verify_signed_blob(
    raw: &[u8],
    bundle_raw: &[u8],
    required_signer: &RequiredSignerV2,
) -> Result<()> {
    let bundle_text = std::str::from_utf8(bundle_raw).context("invalid Sigstore bundle UTF-8")?;
    let rekor_keys = cached_rekor_verification_keys()?;
    let bundle = SignedArtifactBundle::new_verified(bundle_text, rekor_keys.as_ref())
        .map_err(|error| anyhow!("Sigstore Rekor bundle verification failed: {error}"))?;
    let cert_pem = normalize_bundle_cert(&bundle.cert)?;
    <sigstore::cosign::Client as CosignCapabilities>::verify_blob(
        &cert_pem,
        &bundle.base64_signature,
        raw,
    )
    .map_err(|error| anyhow!("Sigstore blob signature verification failed: {error}"))?;
    verify_signer_identity(&cert_pem, required_signer)?;
    Ok(())
}

fn cached_rekor_verification_keys() -> Result<Arc<RekorVerificationKeys>> {
    REKOR_VERIFICATION_KEYS
        .get_or_init(|| {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| format!("failed to build Tokio runtime: {error}"))?;
            let trust_root = runtime
                .block_on(SigstoreTrustRoot::new(None))
                .map_err(|error| format!("failed to load Sigstore trust root: {error}"))?;
            let rekor_keys = trust_root
                .rekor_keys()
                .map_err(|error| format!("failed to load Sigstore Rekor public keys: {error}"))?;
            parse_rekor_verification_keys(rekor_keys)
                .map(Arc::new)
                .map_err(|error| error.to_string())
        })
        .clone()
        .map_err(anyhow::Error::msg)
}

fn parse_rekor_verification_keys(keys: BTreeMap<String, &[u8]>) -> Result<RekorVerificationKeys> {
    let parsed = keys
        .into_iter()
        .filter_map(|(key_id, key)| {
            CosignVerificationKey::from_der(key, &SigningScheme::default())
                .ok()
                .map(|key| (key_id, key))
        })
        .collect::<BTreeMap<_, _>>();
    if parsed.is_empty() {
        bail!("failed to parse any Rekor public keys from the Sigstore trust root");
    }
    Ok(parsed)
}

fn normalize_bundle_cert(cert: &str) -> Result<String> {
    if cert.contains("-----BEGIN CERTIFICATE-----") {
        return Ok(cert.to_string());
    }
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(cert.as_bytes())
        .context("invalid base64 Sigstore certificate")?;
    String::from_utf8(decoded).context("invalid Sigstore certificate UTF-8")
}

fn cert_extension_utf8(cert: &Certificate, oid: &str) -> Result<Option<String>> {
    let Some(extensions) = cert.tbs_certificate.extensions.as_ref() else {
        return Ok(None);
    };
    extensions
        .iter()
        .find(|ext: &&Extension| ext.extn_id.to_string() == oid)
        .map(|ext| {
            String::from_utf8(ext.extn_value.clone().into_bytes())
                .map_err(|_| anyhow!("Sigstore certificate extension {oid} is not valid UTF-8"))
        })
        .transpose()
}

fn cert_subject_uri(cert: &Certificate) -> Result<Option<String>> {
    let san = cert
        .tbs_certificate
        .get::<SubjectAltName>()
        .map_err(|error| anyhow!("failed to read certificate SAN: {error}"))?
        .map(|(_, san)| san);
    let Some(san) = san else {
        return Ok(None);
    };
    Ok(san.0.iter().find_map(|name| match name {
        GeneralName::UniformResourceIdentifier(uri) => Some(uri.to_string()),
        _ => None,
    }))
}

fn verify_signer_identity(cert_pem: &str, required_signer: &RequiredSignerV2) -> Result<()> {
    let cert = Certificate::from_pem(cert_pem.as_bytes())
        .map_err(|error| anyhow!("failed to parse Sigstore certificate: {error}"))?;
    let repository = cert_extension_utf8(&cert, SIGSTORE_GITHUB_WORKFLOW_REPOSITORY_OID)?;
    if repository.as_deref() != Some(required_signer.github_repository.as_str()) {
        bail!(
            "Sigstore signer repo mismatch: expected '{}', got '{}'",
            required_signer.github_repository,
            repository.unwrap_or_else(|| "<missing>".to_string())
        );
    }

    if let Some(expected_workflow) = required_signer.github_workflow.as_deref() {
        let workflow_name = cert_extension_utf8(&cert, SIGSTORE_GITHUB_WORKFLOW_NAME_OID)?;
        let workflow_ref = cert_extension_utf8(&cert, SIGSTORE_GITHUB_WORKFLOW_REF_OID)?;
        let subject_uri = cert_subject_uri(&cert)?;
        let matched = workflow_name.as_deref() == Some(expected_workflow)
            || workflow_ref
                .as_deref()
                .is_some_and(|value| value.contains(expected_workflow))
            || subject_uri
                .as_deref()
                .is_some_and(|value| value.contains(expected_workflow));
        if !matched {
            bail!(
                "Sigstore workflow mismatch for '{}'",
                required_signer.github_repository
            );
        }
    }

    Ok(())
}

fn fetch_verified_bytes(
    _ctx: &TaskContext,
    required_signer: &RequiredSignerV2,
    url: &str,
    bundle_url: &str,
) -> Result<Vec<u8>> {
    let blob_bytes = fetch_url_bytes(url)?;
    let bundle_bytes = fetch_url_bytes(bundle_url)?;
    verify_signed_blob(&blob_bytes, &bundle_bytes, required_signer)?;
    Ok(blob_bytes)
}

fn blake3_hex(bytes: &[u8]) -> String {
    format!("blake3:{}", blake3::hash(bytes).to_hex())
}

fn require_blake3_bytes(label: &str, expected: &str, bytes: &[u8]) -> Result<()> {
    let actual = blake3_hex(bytes);
    if !actual.eq_ignore_ascii_case(expected) {
        bail!("{label} digest mismatch: expected {expected}, got {actual}");
    }
    Ok(())
}

fn latest_catalog_release<'a>(
    plugin_id: &str,
    releases: &'a [ChildCatalogReleaseV2],
) -> Result<&'a ChildCatalogReleaseV2> {
    releases
        .iter()
        .max_by_key(|release| Version::parse(release.version.trim_start_matches('v')).ok())
        .ok_or_else(|| anyhow!("{plugin_id}: child catalog has no releases"))
}

fn manifest_asset_url(manifest_url: &str, asset: &str) -> Result<String> {
    let (base, _) = manifest_url
        .rsplit_once('/')
        .ok_or_else(|| anyhow!("invalid manifest url {manifest_url}"))?;
    Ok(format!("{base}/{asset}"))
}

fn sync_builtin_plugin(ctx: &TaskContext, spec: &BuiltinPluginSpec) -> Result<()> {
    let catalog_signer = RequiredSignerV2 {
        github_repository: OFFICIAL_PLUGIN_REPO.to_string(),
        github_workflow: Some(OFFICIAL_RELEASE_WORKFLOW.to_string()),
    };
    let catalog_bytes = fetch_verified_bytes(
        ctx,
        &catalog_signer,
        OFFICIAL_PLUGIN_CATALOG_URL,
        &bundle_url_for(OFFICIAL_PLUGIN_CATALOG_URL),
    )?;
    let catalog: CatalogV2 = serde_json::from_slice(&catalog_bytes)
        .context("failed to parse official plugin catalog")?;
    let entry = catalog
        .plugins
        .iter()
        .find(|entry| entry.id == spec.plugin_id)
        .ok_or_else(|| {
            anyhow!(
                "builtin plugin '{}' missing from official catalog",
                spec.plugin_id
            )
        })?;
    let child_bytes = fetch_verified_bytes(
        ctx,
        &entry.required_signer,
        &entry.child_catalog_url,
        &bundle_url_for(&entry.child_catalog_url),
    )?;
    let child_bytes = decode_possibly_zstd_bytes(&entry.child_catalog_url, child_bytes)?;
    let child: ChildCatalogV2 = serde_json::from_slice(&child_bytes)
        .with_context(|| format!("failed to parse child catalog for {}", spec.plugin_id))?;
    if child.id != spec.plugin_id {
        bail!(
            "child catalog id mismatch for {}: got {}",
            spec.plugin_id,
            child.id
        );
    }
    let release = latest_catalog_release(spec.plugin_id, &child.releases)?;
    let manifest_bytes = fetch_verified_bytes(
        ctx,
        &entry.required_signer,
        &release.artifact_manifest_url,
        &bundle_url_for(&release.artifact_manifest_url),
    )?;
    let manifest: PluginManifestV2 = serde_json::from_slice(&manifest_bytes)
        .with_context(|| format!("failed to parse manifest for {}", spec.plugin_id))?;
    if manifest.id != spec.plugin_id {
        bail!(
            "manifest id mismatch for {}: got {}",
            spec.plugin_id,
            manifest.id
        );
    }
    let artifact_url = manifest_asset_url(&release.artifact_manifest_url, &manifest.artifact)?;
    let artifact_bundle_url =
        manifest_asset_url(&release.artifact_manifest_url, &manifest.signature)?;
    let compressed_wasm = fetch_verified_bytes(
        ctx,
        &entry.required_signer,
        &artifact_url,
        &artifact_bundle_url,
    )?;
    require_blake3_bytes(
        "compressed builtin artifact",
        &manifest.artifact_digest,
        &compressed_wasm,
    )?;
    if manifest.compression != "zstd" {
        bail!(
            "unsupported builtin artifact compression for {}: {}",
            spec.plugin_id,
            manifest.compression
        );
    }
    let wasm_bytes = zstd::decode_all(compressed_wasm.as_slice()).with_context(|| {
        format!(
            "failed to decompress builtin artifact for {}",
            spec.plugin_id
        )
    })?;
    require_blake3_bytes("builtin wasm", &manifest.wasm_digest, &wasm_bytes)?;
    let descriptor = scryer_plugin_sdk::load_plugin_descriptor_from_wasm_bytes(&wasm_bytes)
        .map_err(|error| anyhow!("failed to describe builtin {}: {error}", spec.plugin_id))?;
    if descriptor.id != spec.plugin_id {
        bail!(
            "descriptor id mismatch for {}: got {}",
            spec.plugin_id,
            descriptor.id
        );
    }
    if descriptor.version != manifest.version {
        bail!(
            "descriptor version mismatch for {}: got {}, expected {}",
            spec.plugin_id,
            descriptor.version,
            manifest.version
        );
    }

    let paths = builtin_asset_paths(ctx, spec);
    for path in [&paths.wasm, &paths.descriptor_json, &paths.description] {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
    }
    fs::write(&paths.wasm, &compressed_wasm)
        .with_context(|| format!("failed to write {}", paths.wasm.display()))?;
    fs::write(
        &paths.descriptor_json,
        serde_json::to_string_pretty(&descriptor)? + "\n",
    )
    .with_context(|| format!("failed to write {}", paths.descriptor_json.display()))?;
    fs::write(
        &paths.description,
        format!("{}\n", child.description.trim()),
    )
    .with_context(|| format!("failed to write {}", paths.description.display()))?;

    ok(format!(
        "synced builtin {} {} from official catalog",
        spec.plugin_id, manifest.version
    ));
    Ok(())
}

fn remove_stale_builtin_assets(ctx: &TaskContext) -> Result<()> {
    let keep = builtin_plugin_paths(ctx)
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
    let dir = ctx.path(BUILTIN_ASSET_DIR);
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let managed = path
            .extension()
            .and_then(OsStr::to_str)
            .is_some_and(|ext| matches!(ext, "wasm" | "json" | "txt"));
        if managed && !keep.contains(&path) {
            fs::remove_file(&path)
                .with_context(|| format!("failed to remove stale builtin {}", path.display()))?;
        }
    }
    Ok(())
}

fn refresh_builtin_plugins(ctx: &TaskContext) -> Result<Vec<PathBuf>> {
    step("Syncing embedded plugin builtins from the official catalog");
    for spec in BUILTIN_PLUGINS {
        sync_builtin_plugin(ctx, spec)
            .with_context(|| format!("failed to sync builtin {}", spec.plugin_id))?;
    }
    remove_stale_builtin_assets(ctx)?;
    ok("Embedded plugin builtins refreshed");
    Ok(builtin_plugin_paths(ctx))
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
        .unwrap_or_else(|_| "rust:1.95.0-bookworm".to_string());
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
    let worktree_clean_at_start = git_status_porcelain(ctx)?.trim().is_empty();
    if !worktree_clean_at_start {
        prompt_continue_if_dirty(ctx)?;
    }
    require_command("gh")?;
    ok("Pre-flight OK");

    let builtin_plugin_paths = builtin_plugin_paths(ctx);
    let initial_cache_dir = release_dry_run_cache_dir(
        ctx,
        &git_commit,
        &release_args,
        latest_tag.as_deref(),
        &next_version,
        &tag_name,
    );
    let initial_cache_dir_relative = relative_to_repo_root(ctx, &initial_cache_dir)?;

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
                worktree_clean_at_start,
                release_args: release_args.clone(),
                latest_tag_seen: latest_tag.clone(),
                next_version: next_version.to_string(),
                tag_name: tag_name.clone(),
                catalog_url: catalog_url.clone(),
                catalog_checksum_sha256: None,
                validated_steps: Vec::new(),
                cached_builtins_dir: Some(initial_cache_dir_relative.clone()),
                failure_message: Some("dry run did not complete".to_string()),
            },
        )?;
    } else if worktree_clean_at_start && release_dry_run_cache_path(ctx).is_file() {
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
        let validation_result = {
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
            Ok::<(Vec<PathBuf>, Vec<String>), anyhow::Error>((
                refreshed_builtin_paths,
                vec![
                    "builtin_refresh".to_string(),
                    "web_validation".to_string(),
                    "rust_validation".to_string(),
                ],
            ))
        };

        if args.dry_run {
            match validation_result {
                Ok((refreshed_builtin_paths, validated_steps)) => {
                    let prep_changed_paths = git_tracked_dirty_paths(ctx)?;
                    let final_git_commit = if !prep_changed_paths.is_empty() {
                        step("Committing release-prep changes");
                        let committed = commit_tracked_changes(
                            ctx,
                            &prep_changed_paths,
                            &format!("release: prep scryer {next_version}"),
                        )?
                        .expect("non-empty tracked changes should produce a commit");
                        ok(format!("Committed release-prep changes in {committed}"));
                        committed
                    } else {
                        ok("No release-prep changes to commit");
                        git_commit.clone()
                    };
                    let final_cache_dir = release_dry_run_cache_dir(
                        ctx,
                        &final_git_commit,
                        &release_args,
                        latest_tag.as_deref(),
                        &next_version,
                        &tag_name,
                    );
                    let final_cache_dir_relative = relative_to_repo_root(ctx, &final_cache_dir)?;
                    let catalog_checksum_sha256 = fetch_catalog_checksum(&catalog_url)?;
                    cache_builtin_artifacts(&final_cache_dir, &refreshed_builtin_paths)?;
                    write_release_dry_run_cache(
                        ctx,
                        &ReleaseDryRunCache {
                            success: true,
                            created_at: Utc::now().to_rfc3339(),
                            git_commit: final_git_commit,
                            branch: branch.clone(),
                            worktree_clean_at_start,
                            release_args: release_args.clone(),
                            latest_tag_seen: latest_tag.clone(),
                            next_version: next_version.to_string(),
                            tag_name: tag_name.clone(),
                            catalog_url: catalog_url.clone(),
                            catalog_checksum_sha256: Some(catalog_checksum_sha256),
                            validated_steps,
                            cached_builtins_dir: Some(final_cache_dir_relative),
                            failure_message: None,
                        },
                    )?;
                    println!(
                        "\n{YELLOW}{BOLD}Dry run complete — stopping before commit/tag/push.{RESET}"
                    );
                    println!("  Version {next_version} validated OK.");
                    println!(
                        "  Dry-run cache: {}",
                        release_dry_run_cache_path(ctx).display()
                    );
                    return Ok(());
                }
                Err(error) => {
                    return Err(error);
                }
            }
        }

        let _ = validation_result?;
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
    prefixed_step(prefix, "Running cargo fmt --all");
    let mut fmt_fix = ctx.release_command_in("cargo", &ctx.repo_root);
    fmt_fix.args(["fmt", "--all"]);
    run_streaming(&mut fmt_fix, prefix)?;
    prefixed_ok(prefix, "cargo fmt complete");

    prefixed_step(prefix, "Running cargo fmt --all --check");
    let mut fmt = ctx.release_command_in("cargo", &ctx.repo_root);
    fmt.args(["fmt", "--all", "--check"]);
    run_streaming(&mut fmt, prefix)?;
    prefixed_ok(prefix, "cargo fmt passed");

    prefixed_step(
        prefix,
        "Running cargo clippy --fix for scryer production binary packages",
    );
    let mut clippy_fix = ctx.release_command_in("cargo", &ctx.repo_root);
    clippy_fix.arg("clippy");
    add_prod_package_args(&mut clippy_fix);
    clippy_fix.args([
        "--fix",
        "--allow-dirty",
        "--allow-staged",
        "--",
        "-D",
        "warnings",
    ]);
    run_streaming(&mut clippy_fix, prefix)?;
    prefixed_ok(prefix, "cargo clippy --fix complete");

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
            worktree_clean_at_start: true,
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
    fn release_dry_run_cache_allows_dirty_start_when_other_inputs_match() {
        let mut cache = sample_release_dry_run_cache();
        cache.worktree_clean_at_start = false;
        let reason = release_dry_run_cache_rejection_reason(
            &cache,
            &sample_release_dry_run_expectations(),
            true,
        );
        assert!(reason.is_none());
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
