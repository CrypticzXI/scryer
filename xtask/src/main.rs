use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use chrono::Utc;
use clap::{Args, Parser, Subcommand};
use sha2::{Digest, Sha256};
#[cfg(unix)]
use signal_hook::consts::signal::{SIGINT, SIGTERM};
#[cfg(unix)]
use signal_hook::iterator::{Handle as SignalHandle, Signals};
use std::fs;
use std::io::{self, Write};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use tempfile::NamedTempFile;
use xtask_support::{TaskContext, command_available, ok, run_status, step, warn};

mod profile;
mod seed;

const BACKEND_SHUTDOWN_GRACE_PERIOD: std::time::Duration = std::time::Duration::from_secs(5);
const DEFAULT_SERVE_BIND: &str = "127.0.0.1:18080";
const DEFAULT_SERVE_FRONTEND_PORT: u16 = 3000;

#[cfg(unix)]
struct SignalForwarder {
    handle: SignalHandle,
    process_groups: Arc<Mutex<Vec<u32>>>,
    shutdown_requested: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

#[cfg(unix)]
impl Drop for SignalForwarder {
    fn drop(&mut self) {
        self.handle.close();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(not(unix))]
struct SignalForwarder;

#[cfg(unix)]
impl SignalForwarder {
    fn replace_process_groups(&self, process_ids: &[u32]) {
        if let Ok(mut groups) = self.process_groups.lock() {
            groups.clear();
            groups.extend(process_ids.iter().copied());
        }
    }

    fn shutdown_requested(&self) -> bool {
        self.shutdown_requested.load(Ordering::SeqCst)
    }
}

#[cfg(not(unix))]
impl SignalForwarder {
    fn replace_process_groups(&self, _process_ids: &[u32]) {}

    fn shutdown_requested(&self) -> bool {
        false
    }
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
    Migrations(MigrationsArgs),
    Sdk(SdkArgs),
    ValidateTrashGuides,
    Ci(CiArgs),
    Stack(StackArgs),
    Serve(ServeArgs),
    Seed(SeedArgs),
    Profile(ProfileArgs),
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
struct BuiltinsArgs {
    #[command(subcommand)]
    command: BuiltinsCommand,
}

#[derive(Subcommand)]
enum BuiltinsCommand {
    Sync,
}

#[derive(Args)]
struct MigrationsArgs {
    #[command(subcommand)]
    command: MigrationsCommand,
}

#[derive(Subcommand)]
enum MigrationsCommand {
    Rebaseline(RebaselineArgs),
}

#[derive(Args, Clone)]
struct RebaselineArgs {
    #[arg(long)]
    through: i64,
    #[arg(long)]
    force: bool,
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
struct ServeArgs {
    #[arg(
        long,
        default_value = DEFAULT_SERVE_BIND,
        help = "Bind address for the locally hosted Scryer debug server"
    )]
    bind: String,
    #[arg(
        long,
        default_value_t = DEFAULT_SERVE_FRONTEND_PORT,
        help = "Port for the Vite dev server with hot reload"
    )]
    frontend_port: u16,
    #[arg(
        long,
        help = "Run xtask serve against a managed PostgreSQL Docker container instead of the default SQLite datastore"
    )]
    postgres: bool,
    #[arg(long, help = "Reset the selected datastore before starting Scryer")]
    clean: bool,
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

#[derive(Clone, Copy)]
enum ServeMode {
    PreserveDatabase,
    CleanDatabase,
}

#[derive(Clone, Copy)]
enum ServeDatastoreKind {
    Sqlite,
    Postgres,
}

struct ServeDatastore {
    kind: ServeDatastoreKind,
    envs: Vec<(String, String)>,
    location: String,
}

struct ServePostgresConfig {
    image: String,
    container_name: String,
    volume_name: String,
    host_port: u16,
    database: String,
    user: String,
    password: String,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let ctx = TaskContext::new();

    match cli.command {
        Commands::Release(args) => delegate_release(&ctx, &args),
        Commands::Builtins(args) => delegate_builtins(&ctx, &args),
        Commands::Migrations(args) => delegate_migrations(&ctx, &args),
        Commands::Sdk(args) => delegate_sdk(&ctx, &args),
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
        Commands::Serve(args) => {
            let mode = if args.clean {
                ServeMode::CleanDatabase
            } else {
                ServeMode::PreserveDatabase
            };
            serve_local_scryer(&ctx, args, mode)
        }
        Commands::Seed(args) => match args.command {
            SeedCommand::Dev(args) => seed_dev(&ctx, args),
        },
        Commands::Profile(args) => match args.command {
            ProfileCommand::Hotpaths(args) => profile_hotpaths(&ctx, args),
        },
    }
}

fn delegate_release(ctx: &TaskContext, args: &ReleaseArgs) -> Result<()> {
    let mut forwarded = vec!["release".to_string()];
    if args.major {
        forwarded.push("--major".to_string());
    }
    if args.minor {
        forwarded.push("--minor".to_string());
    }
    if args.patch {
        forwarded.push("--patch".to_string());
    }
    if args.dry_run {
        forwarded.push("--dry-run".to_string());
    }
    if let Some(version) = &args.version {
        forwarded.push(version.clone());
    }
    delegate_to_package(ctx, "xtask-release", &forwarded)
}

fn delegate_builtins(ctx: &TaskContext, args: &BuiltinsArgs) -> Result<()> {
    let forwarded = match args.command {
        BuiltinsCommand::Sync => vec!["builtins".to_string(), "sync".to_string()],
    };
    delegate_to_package(ctx, "xtask-release", &forwarded)
}

fn delegate_sdk(ctx: &TaskContext, args: &SdkArgs) -> Result<()> {
    let mut forwarded = vec!["sdk".to_string()];
    match &args.command {
        SdkCommand::Release(release) => {
            forwarded.push("release".to_string());
            forwarded.push(release.version.clone());
            if release.dry_run {
                forwarded.push("--dry-run".to_string());
            }
        }
    }
    delegate_to_package(ctx, "xtask-release", &forwarded)
}

fn delegate_migrations(ctx: &TaskContext, args: &MigrationsArgs) -> Result<()> {
    let mut forwarded = Vec::new();
    match &args.command {
        MigrationsCommand::Rebaseline(rebaseline) => {
            forwarded.push("rebaseline".to_string());
            forwarded.push("--through".to_string());
            forwarded.push(rebaseline.through.to_string());
            if rebaseline.force {
                forwarded.push("--force".to_string());
            }
        }
    }
    delegate_to_package(ctx, "xtask-migrations", &forwarded)
}

fn delegate_to_package(ctx: &TaskContext, package: &str, forwarded: &[String]) -> Result<()> {
    let mut command = ctx.command_in("cargo", &ctx.repo_root);
    command
        .arg("run")
        .arg("--locked")
        .arg("-p")
        .arg(package)
        .arg("--")
        .args(forwarded);
    run_checked(&mut command)
}

fn seed_dev(ctx: &TaskContext, args: SeedDevArgs) -> Result<()> {
    seed::run(ctx, args)
}

fn profile_hotpaths(ctx: &TaskContext, args: ProfileHotpathsArgs) -> Result<()> {
    profile::run(ctx, args)
}

fn tail_file(path: &Path, lines: usize) -> Result<String> {
    let content = fs::read_to_string(path).unwrap_or_default();
    let collected = content.lines().rev().take(lines).collect::<Vec<_>>();
    Ok(collected.into_iter().rev().collect::<Vec<_>>().join("\n"))
}

enum BackendStartupOutcome {
    Ready,
    Interrupted,
}

fn wait_for_local_backend(
    backend: &mut std::process::Child,
    port: u16,
    log_path: &Path,
    signal_forwarder: &SignalForwarder,
) -> Result<BackendStartupOutcome> {
    let address = format!("127.0.0.1:{port}");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
    while std::time::Instant::now() < deadline {
        if signal_forwarder.shutdown_requested() {
            return Ok(BackendStartupOutcome::Interrupted);
        }

        if let Some(status) = backend.try_wait()? {
            let tail = tail_file(log_path, 50)?;
            bail!(
                "Scryer failed to start on http://{address}/ (status: {status}). Tail of {}:\n{tail}",
                log_path.display()
            );
        }

        if backend_ready_looks_ok(port) {
            return Ok(BackendStartupOutcome::Ready);
        }

        thread::sleep(std::time::Duration::from_millis(250));
    }

    let tail = tail_file(log_path, 50)?;
    bail!(
        "Timed out waiting for Scryer readiness on http://{address}/graphql. Tail of {}:\n{tail}",
        log_path.display()
    )
}

enum ServeWaitOutcome {
    FrontendExited(std::process::ExitStatus),
    Interrupted,
}

fn wait_for_serve_processes(
    backend: &mut Child,
    frontend: &mut Child,
    backend_log_path: &Path,
    signal_forwarder: &SignalForwarder,
) -> Result<ServeWaitOutcome> {
    loop {
        if signal_forwarder.shutdown_requested() {
            return Ok(ServeWaitOutcome::Interrupted);
        }

        if let Some(status) = backend.try_wait()? {
            let tail = tail_file(backend_log_path, 50)?;
            bail!(
                "Scryer backend exited while xtask serve was running (status: {status}). Tail of {}:\n{tail}",
                backend_log_path.display()
            );
        }

        if let Some(status) = frontend.try_wait()? {
            return Ok(ServeWaitOutcome::FrontendExited(status));
        }

        thread::sleep(std::time::Duration::from_millis(100));
    }
}

fn backend_ready_looks_ok(port: u16) -> bool {
    backend_health_looks_ok(port) && backend_graphql_looks_ready(port)
}

fn backend_health_looks_ok(port: u16) -> bool {
    http_request(
        port,
        &format!(
            "GET /health HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAccept: application/json\r\nConnection: close\r\n\r\n"
        ),
    )
    .and_then(|(status_line, body)| {
        if !status_line.contains(" 200 ") {
            return None;
        }
        serde_json::from_str::<serde_json::Value>(&body).ok()
    })
    .and_then(|payload| {
        payload
            .get("status")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    })
    .as_deref()
        == Some("ok")
}

fn backend_graphql_looks_ready(port: u16) -> bool {
    let body = r#"{"query":"query { authRuntimeState { effectiveFormLoginEnabled skipLoginForLocalIps } }"}"#;
    let request = format!(
        "POST /graphql HTTP/1.1\r\n\
Host: 127.0.0.1:{port}\r\n\
Accept: application/json\r\n\
Content-Type: application/json\r\n\
Content-Length: {content_length}\r\n\
Connection: close\r\n\r\n\
{body}",
        content_length = body.len(),
    );

    http_request(port, &request)
        .and_then(|(status_line, body)| {
            if !status_line.contains(" 200 ") {
                return None;
            }
            serde_json::from_str::<serde_json::Value>(&body).ok()
        })
        .and_then(|payload| {
            payload
                .get("data")
                .and_then(|data| data.get("authRuntimeState"))
                .cloned()
        })
        .is_some()
}

fn http_request(port: u16, request: &str) -> Option<(String, String)> {
    let Ok(mut stream) = std::net::TcpStream::connect(("127.0.0.1", port)) else {
        return None;
    };
    let timeout = Some(std::time::Duration::from_millis(500));
    if stream.set_read_timeout(timeout).is_err() || stream.set_write_timeout(timeout).is_err() {
        return None;
    }
    if write!(stream, "{request}").is_err() {
        return None;
    }

    let mut response = String::new();
    if std::io::Read::read_to_string(&mut stream, &mut response).is_err() {
        return None;
    }

    let Some((headers, body)) = response.split_once("\r\n\r\n") else {
        return None;
    };
    let mut header_lines = headers.lines();
    let Some(status_line) = header_lines.next() else {
        return None;
    };

    let body = if headers
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked")
    {
        decode_chunked_http_body(body)?
    } else {
        body.to_string()
    };

    Some((status_line.to_string(), body))
}

fn decode_chunked_http_body(body: &str) -> Option<String> {
    let mut decoded = String::new();
    let mut rest = body;

    loop {
        let (size_line, after_size_line) = rest.split_once("\r\n")?;
        let size = usize::from_str_radix(size_line.trim(), 16).ok()?;
        if size == 0 {
            return Some(decoded);
        }
        if after_size_line.len() < size + 2 {
            return None;
        }
        decoded.push_str(&after_size_line[..size]);
        rest = &after_size_line[size + 2..];
    }
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
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|home| home.join(".local/share"))
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

fn reset_serve_database(db_path: &Path) -> Result<()> {
    let cleanup_targets = [
        db_path.to_path_buf(),
        PathBuf::from(format!("{}-wal", db_path.display())),
        PathBuf::from(format!("{}-shm", db_path.display())),
    ];

    step(format!(
        "Removing xtask serve database files under {}",
        db_path.display()
    ));
    for path in cleanup_targets {
        if path.exists() {
            fs::remove_file(&path)
                .with_context(|| format!("failed to remove {}", path.display()))?;
        }
    }
    ok("xtask serve database reset");
    Ok(())
}

fn serve_postgres_config() -> Result<ServePostgresConfig> {
    let host_port = std::env::var("SCRYER_XTASK_POSTGRES_PORT")
        .ok()
        .map(|value| {
            value.parse::<u16>().with_context(|| {
                format!("SCRYER_XTASK_POSTGRES_PORT must be a valid port, got {value}")
            })
        })
        .transpose()?
        .unwrap_or(55432);
    Ok(ServePostgresConfig {
        image: std::env::var("SCRYER_XTASK_POSTGRES_IMAGE")
            .unwrap_or_else(|_| "postgres:18".to_string()),
        container_name: std::env::var("SCRYER_XTASK_POSTGRES_CONTAINER")
            .unwrap_or_else(|_| "scryer-xtask-postgres".to_string()),
        volume_name: std::env::var("SCRYER_XTASK_POSTGRES_VOLUME")
            .unwrap_or_else(|_| "scryer-xtask-postgres-data".to_string()),
        host_port,
        database: std::env::var("SCRYER_XTASK_POSTGRES_DB")
            .unwrap_or_else(|_| "scryer".to_string()),
        user: std::env::var("SCRYER_XTASK_POSTGRES_USER").unwrap_or_else(|_| "scryer".to_string()),
        password: std::env::var("SCRYER_XTASK_POSTGRES_PASSWORD")
            .unwrap_or_else(|_| "scryer-dev-password".to_string()),
    })
}

fn docker_container_exists(ctx: &TaskContext, container_name: &str) -> Result<bool> {
    let mut inspect = ctx.command("docker");
    inspect.args(["container", "inspect", container_name]);
    Ok(inspect.output()?.status.success())
}

fn docker_volume_exists(ctx: &TaskContext, volume_name: &str) -> Result<bool> {
    let mut inspect = ctx.command("docker");
    inspect.args(["volume", "inspect", volume_name]);
    Ok(inspect.output()?.status.success())
}

fn reset_serve_postgres(ctx: &TaskContext, config: &ServePostgresConfig) -> Result<()> {
    step(format!(
        "Resetting xtask serve PostgreSQL container {} and volume {}",
        config.container_name, config.volume_name
    ));
    if docker_container_exists(ctx, &config.container_name)? {
        let mut rm = ctx.command("docker");
        rm.args(["rm", "-f", &config.container_name]);
        run_checked(&mut rm)?;
    }
    if docker_volume_exists(ctx, &config.volume_name)? {
        let mut rm = ctx.command("docker");
        rm.args(["volume", "rm", "-f", &config.volume_name]);
        run_checked(&mut rm)?;
    }
    ok("xtask serve PostgreSQL state reset");
    Ok(())
}

fn wait_for_serve_postgres(ctx: &TaskContext, config: &ServePostgresConfig) -> Result<()> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while std::time::Instant::now() < deadline {
        if matches!(
            docker_inspect_state(&config.container_name)?.as_deref(),
            Some("exited" | "dead")
        ) {
            warn(format!(
                "PostgreSQL container {} exited before becoming ready",
                config.container_name
            ));
            log_container_failure(&config.container_name)?;
            bail!(
                "PostgreSQL container {} exited before becoming ready",
                config.container_name
            );
        }

        let pg_url = format!(
            "postgresql://{}@127.0.0.1/{}?sslmode=disable",
            config.user, config.database
        );
        let mut psql = ctx.command("docker");
        psql.args([
            "exec",
            &config.container_name,
            "env",
            &format!("PGPASSWORD={}", config.password),
            "psql",
            &pg_url,
            "-c",
            "SELECT 1",
        ]);
        if run_status(&mut psql)?.success() {
            return Ok(());
        }

        thread::sleep(std::time::Duration::from_millis(500));
    }

    warn(format!(
        "Timed out waiting for PostgreSQL container {} to become ready",
        config.container_name
    ));
    log_container_failure(&config.container_name)?;
    bail!(
        "Timed out waiting for PostgreSQL container {} to become ready",
        config.container_name
    );
}

fn ensure_serve_postgres(ctx: &TaskContext, mode: ServeMode) -> Result<ServeDatastore> {
    require_command("docker")?;
    let config = serve_postgres_config()?;

    if matches!(mode, ServeMode::CleanDatabase) {
        reset_serve_postgres(ctx, &config)?;
    }

    if docker_container_exists(ctx, &config.container_name)? {
        let state = docker_inspect_state(&config.container_name)?;
        if !matches!(state.as_deref(), Some("running")) {
            step(format!(
                "Starting managed PostgreSQL container {}",
                config.container_name
            ));
            let mut start = ctx.command("docker");
            start.args(["start", &config.container_name]);
            run_checked(&mut start)?;
        } else {
            ok(format!(
                "Reusing managed PostgreSQL container {}",
                config.container_name
            ));
        }
    } else {
        step(format!(
            "Creating managed PostgreSQL container {} from {}",
            config.container_name, config.image
        ));
        let mut run = ctx.command("docker");
        run.args([
            "run",
            "-d",
            "--name",
            &config.container_name,
            "-e",
            &format!("POSTGRES_DB={}", config.database),
            "-e",
            &format!("POSTGRES_USER={}", config.user),
            "-e",
            &format!("POSTGRES_PASSWORD={}", config.password),
            "-p",
            &format!("{}:5432", config.host_port),
            "-v",
            &format!("{}:/var/lib/postgresql", config.volume_name),
            &config.image,
        ]);
        run_checked(&mut run)?;
    }

    step(format!(
        "Waiting for PostgreSQL on 127.0.0.1:{}",
        config.host_port
    ));
    wait_for_serve_postgres(ctx, &config)?;
    ok(format!(
        "Managed PostgreSQL is ready in container {}",
        config.container_name
    ));

    Ok(ServeDatastore {
        kind: ServeDatastoreKind::Postgres,
        envs: vec![
            (
                "SCRYER_DB_URL".to_string(),
                format!(
                    "postgres://127.0.0.1:{}/{}?sslmode=disable",
                    config.host_port, config.database
                ),
            ),
            ("SCRYER_DB_USER".to_string(), config.user.clone()),
            ("SCRYER_DB_PASSWORD".to_string(), config.password.clone()),
        ],
        location: format!(
            "postgres://127.0.0.1:{}/{}?sslmode=disable (container={}, volume={})",
            config.host_port, config.database, config.container_name, config.volume_name
        ),
    })
}

fn prepare_serve_datastore(
    ctx: &TaskContext,
    args: &ServeArgs,
    mode: ServeMode,
) -> Result<ServeDatastore> {
    if args.postgres {
        return ensure_serve_postgres(ctx, mode);
    }

    let db_path = serve_db_path()?;
    if matches!(mode, ServeMode::CleanDatabase) {
        reset_serve_database(&db_path)?;
    }
    let db_url = format!("sqlite://{}", db_path.display());
    Ok(ServeDatastore {
        kind: ServeDatastoreKind::Sqlite,
        envs: vec![("SCRYER_DB_PATH".to_string(), db_url)],
        location: db_path.display().to_string(),
    })
}

fn serve_encryption_key() -> String {
    let digest = Sha256::digest(b"scryer-xtask-dev-encryption-key");
    base64::engine::general_purpose::STANDARD.encode(digest)
}

fn ensure_frontend_dependencies(ctx: &TaskContext, web_dir: &Path) -> Result<()> {
    step("Syncing frontend dependencies for Vite dev server");
    let mut install = ctx.command_in("npm", web_dir);
    install.args(["install", "--no-fund", "--no-audit"]);
    run_status(&mut install).with_context(|| {
        format!(
            "failed to install frontend dependencies in {}",
            web_dir.display()
        )
    })?;
    ok("Frontend dependencies are up to date");
    Ok(())
}

fn serve_local_scryer(ctx: &TaskContext, args: ServeArgs, mode: ServeMode) -> Result<()> {
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
    ensure_frontend_dependencies(ctx, &web_dir)?;

    let backend_port = backend_port(&args.bind)?;
    let frontend_port = resolve_frontend_port(args.frontend_port)?;
    let backend_url = format!("http://127.0.0.1:{backend_port}");
    let frontend_url = format!("http://127.0.0.1:{frontend_port}");
    let vite_use_polling =
        std::env::var("SCRYER_VITE_USE_POLLING").unwrap_or_else(|_| "true".to_string());
    let vite_poll_interval =
        std::env::var("SCRYER_VITE_POLL_INTERVAL_MS").unwrap_or_else(|_| "250".to_string());
    let datastore = prepare_serve_datastore(ctx, &args, mode)?;
    let encryption_key = serve_encryption_key();
    let backend_binary = ctx.path("target/debug/scryer");
    let backend_log = PathBuf::from(
        std::env::var("SCRYER_DEV_BACKEND_LOG")
            .unwrap_or_else(|_| "/tmp/scryer-dev-backend.log".to_string()),
    );
    if let Some(parent) = backend_log.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&backend_log, "")?;

    step("Building Scryer backend");
    let mut build = ctx.command_in("cargo", &ctx.repo_root);
    build.args(["build", "--locked", "-p", "scryer"]);
    run_checked(&mut build)?;

    step(format!(
        "Starting Scryer backend from {} on {}",
        backend_binary.display(),
        args.bind
    ));
    if env_file.is_file() {
        ok(format!(
            "Loaded runtime environment from {}",
            env_file.display()
        ));
    }
    if frontend_port != args.frontend_port {
        warn(format!(
            "frontend port {} is busy; using {} for the Vite dev server",
            args.frontend_port, frontend_port
        ));
    }
    println!("   Vite dev server: {frontend_url}");
    println!("   Vite file watch: polling={vite_use_polling} interval_ms={vite_poll_interval}");
    println!("   Keychain: disabled for xtask serve");
    match datastore.kind {
        ServeDatastoreKind::Sqlite => println!("   Datastore: SQLite ({})", datastore.location),
        ServeDatastoreKind::Postgres => {
            println!("   Datastore: PostgreSQL ({})", datastore.location)
        }
    }

    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&backend_log)?;
    let log_err = log.try_clone()?;
    let mut serve = ctx.command(&backend_binary);
    configure_child_process_group(&mut serve);
    for (key, value) in &dotenv_envs {
        serve.env(key, value);
    }
    serve
        .env_remove("SCRYER_DB_URL")
        .env_remove("SCRYER_DB_PATH")
        .env_remove("SCRYER_DB_USER")
        .env_remove("SCRYER_DB_PASSWORD")
        .env_remove("SCRYER_DB_PASSWORD_FILE");
    for (key, value) in &datastore.envs {
        serve.env(key, value);
    }
    serve
        .env("SCRYER_DISABLE_PLATFORM_KEYSTORE", "1")
        .env("SCRYER_ENCRYPTION_KEY", &encryption_key)
        .env("SCRYER_OPEN_BROWSER", "false")
        .env("SCRYER_WEB_UI_URL", &frontend_url)
        .env("SCRYER_BIND", &args.bind)
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log_err));
    let mut backend = serve.spawn()?;
    let signal_forwarder = install_signal_forwarder(&[backend.id()])?;
    match wait_for_local_backend(&mut backend, backend_port, &backend_log, &signal_forwarder) {
        Ok(BackendStartupOutcome::Ready) => {}
        Ok(BackendStartupOutcome::Interrupted) => {
            drop(signal_forwarder);
            terminate_child_process_group(&mut backend);
            return Ok(());
        }
        Err(error) => {
            drop(signal_forwarder);
            terminate_child_process_group(&mut backend);
            return Err(error);
        }
    }

    println!("==> Scryer backend ready");
    println!("    Backend:  {backend_url}");
    println!("    Frontend: {frontend_url}");
    println!("    Datastore: {}", datastore.location);
    println!("    Log:      tail -f {}", backend_log.display());
    println!();
    println!("==> Starting Vite dev server with live updates...");

    let mut vite = ctx.command_in("npm", &web_dir);
    configure_child_process_group(&mut vite);
    vite.env("SCRYER_DEV_PROXY_TARGET", &backend_url)
        .env("SCRYER_VITE_USE_POLLING", &vite_use_polling)
        .env("SCRYER_VITE_POLL_INTERVAL_MS", &vite_poll_interval)
        .args([
            "run",
            "dev",
            "--",
            "--host",
            "0.0.0.0",
            "--strictPort",
            "--port",
            &frontend_port.to_string(),
        ]);
    let mut vite = vite.spawn()?;
    signal_forwarder.replace_process_groups(&[backend.id(), vite.id()]);

    let result = wait_for_serve_processes(&mut backend, &mut vite, &backend_log, &signal_forwarder);

    drop(signal_forwarder);
    terminate_child_process_group(&mut vite);
    terminate_child_process_group(&mut backend);

    match result? {
        ServeWaitOutcome::Interrupted => Ok(()),
        ServeWaitOutcome::FrontendExited(status) if status.success() => Ok(()),
        ServeWaitOutcome::FrontendExited(status) => {
            bail!("Vite dev server exited with status {status}")
        }
    }
}

#[cfg(unix)]
fn configure_child_process_group(command: &mut Command) {
    unsafe {
        command.pre_exec(|| {
            if libc::setpgid(0, 0) == 0 {
                Ok(())
            } else {
                Err(io::Error::last_os_error())
            }
        });
    }
}

#[cfg(not(unix))]
fn configure_child_process_group(_command: &mut Command) {}

#[cfg(unix)]
fn install_signal_forwarder(process_ids: &[u32]) -> Result<SignalForwarder> {
    let mut signals = Signals::new([SIGINT, SIGTERM])?;
    let handle = signals.handle();
    let process_groups = Arc::new(Mutex::new(process_ids.to_vec()));
    let process_groups_for_thread = Arc::clone(&process_groups);
    let shutdown_requested = Arc::new(AtomicBool::new(false));
    let shutdown_requested_for_thread = Arc::clone(&shutdown_requested);
    let thread = thread::spawn(move || {
        for signal in signals.forever() {
            shutdown_requested_for_thread.store(true, Ordering::SeqCst);
            let process_groups = process_groups_for_thread
                .lock()
                .map(|groups| groups.clone())
                .unwrap_or_default();
            for process_id in process_groups {
                let _ = signal_process_group(process_id, signal);
            }
        }
    });
    Ok(SignalForwarder {
        handle,
        process_groups,
        shutdown_requested,
        thread: Some(thread),
    })
}

#[cfg(not(unix))]
fn install_signal_forwarder(_process_ids: &[u32]) -> Result<SignalForwarder> {
    Ok(SignalForwarder)
}

fn terminate_child_process_group(child: &mut Child) {
    #[cfg(unix)]
    {
        let process_id = child.id();
        let _ = signal_process_group(process_id, SIGINT);
        if wait_for_child_exit(child, BACKEND_SHUTDOWN_GRACE_PERIOD) {
            return;
        }
        let _ = signal_process_group(process_id, libc::SIGKILL);
        let _ = child.wait();
    }

    #[cfg(not(unix))]
    {
        let _ = child.kill();
        let _ = child.wait();
    }
}

#[cfg(unix)]
fn wait_for_child_exit(backend: &mut Child, timeout: std::time::Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match backend.try_wait() {
            Ok(Some(_)) => return true,
            Ok(None) if std::time::Instant::now() < deadline => {
                thread::sleep(std::time::Duration::from_millis(100));
            }
            Ok(None) => return false,
            Err(_) => return false,
        }
    }
}

#[cfg(unix)]
fn signal_process_group(process_id: u32, signal: i32) -> io::Result<()> {
    let process_group = i32::try_from(process_id)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "process id overflow"))?;
    let result = unsafe { libc::kill(-process_group, signal) };
    if result == 0 {
        return Ok(());
    }

    let error = io::Error::last_os_error();
    if matches!(error.raw_os_error(), Some(libc::ESRCH)) {
        return Ok(());
    }
    Err(error)
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
                "set -euo pipefail; /usr/local/cargo/bin/rustup component add clippy; toolchain=\"$('/usr/local/cargo/bin/rustup' show active-toolchain | cut -d' ' -f1)\"; toolchain_bin=\"/usr/local/rustup/toolchains/${toolchain}/bin\"; export PATH=\"${toolchain_bin}:$PATH\"; \"${toolchain_bin}/cargo-clippy\" clippy --workspace --locked -- -D warnings",
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

pub(crate) fn require_command(command: &str) -> Result<()> {
    xtask_support::require_command(command)
}

pub(crate) fn run_checked(command: &mut Command) -> Result<()> {
    xtask_support::run_checked(command)
}

pub(crate) fn run_capture(command: &mut Command) -> Result<String> {
    xtask_support::run_capture(command)
}
