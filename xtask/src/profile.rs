use crate::{ProfileHotpathsArgs, TaskContext};
use anyhow::{Context, Result, bail};
use chrono::Utc;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub(crate) fn run(ctx: &TaskContext, args: ProfileHotpathsArgs) -> Result<()> {
    crate::require_command("docker")?;

    let container = env::var("SCRYER_PROFILE_CONTAINER").unwrap_or_else(|_| "scryer".to_string());
    let duration_seconds = args
        .duration_seconds
        .unwrap_or_else(|| "20".to_string())
        .parse::<f64>()
        .context("duration must be a number of seconds")?;
    let interval_seconds = args
        .interval_seconds
        .unwrap_or_else(|| "0.5".to_string())
        .parse::<f64>()
        .context("interval must be a number of seconds")?;
    let sample_depth = env::var("SCRYER_PROFILE_SAMPLE_DEPTH")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(12);
    let out_dir = env::var("SCRYER_PROFILE_OUT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp/scryer-hotpaths"));

    let timestamp = Utc::now().format("%Y%m%d-%H%M%S").to_string();
    let run_dir = out_dir.join(&timestamp);
    let raw_dir = run_dir.join("raw");
    fs::create_dir_all(&raw_dir)?;

    require_running_container(ctx, &container)?;
    validate_tools(ctx, &container)?;

    let pid = find_scryer_pid(ctx, &container)?;
    if pid.is_empty() {
        bail!("could not find a running scryer pid inside {container}");
    }

    println!("Validating debugger attach to {container} pid {pid}...");
    validate_attach(ctx, &container, &pid, &run_dir, sample_depth)?;

    println!("Sampling {container} pid {pid} every {interval_seconds}s for {duration_seconds}s...");
    let deadline = Instant::now() + Duration::from_secs_f64(duration_seconds);
    let interval = Duration::from_secs_f64(interval_seconds);
    let mut sample_idx = 0usize;
    while Instant::now() < deadline {
        sample_idx += 1;
        take_sample(ctx, &container, &pid, sample_idx, sample_depth, &raw_dir)?;
        std::thread::sleep(interval);
    }

    let summary = summarize_samples(
        &container,
        duration_seconds,
        interval_seconds,
        sample_depth,
        &raw_dir,
    );
    let summary_file = run_dir.join("summary.txt");
    fs::write(&summary_file, summary)?;

    println!("Profile complete.");
    println!("Summary: {}", summary_file.display());
    println!("Raw samples: {}", raw_dir.display());
    Ok(())
}

fn require_running_container(ctx: &TaskContext, container: &str) -> Result<()> {
    let mut command = ctx.command("docker");
    command.args(["inspect", container, "--format", "{{.State.Status}}"]);
    let output = crate::run_capture(&mut command).unwrap_or_default();
    if output.trim() == "running" {
        Ok(())
    } else {
        bail!("container {container} is not running")
    }
}

fn validate_tools(ctx: &TaskContext, container: &str) -> Result<()> {
    docker_exec_checked(
        ctx,
        container,
        "command -v gdb >/dev/null && command -v ps >/dev/null",
    )
    .context("container is missing required tools (need gdb and ps)")
}

fn find_scryer_pid(ctx: &TaskContext, container: &str) -> Result<String> {
    let first = docker_exec_capture(
        ctx,
        container,
        "ps -eo pid,args | awk '/\\/cargo-target\\/debug\\/scryer|target\\/debug\\/scryer/ && !/awk/ { print $1; exit }'",
    )?;
    let pid = first.trim().to_string();
    if !pid.is_empty() {
        return Ok(pid);
    }

    Ok(docker_exec_capture(
        ctx,
        container,
        "ps -eo pid,args | awk '/cargo run --locked -p scryer/ && !/awk/ { print $1; exit }'",
    )?
    .trim()
    .to_string())
}

fn validate_attach(
    ctx: &TaskContext,
    container: &str,
    pid: &str,
    run_dir: &Path,
    sample_depth: usize,
) -> Result<()> {
    let output = docker_exec_output(
        ctx,
        container,
        &format!(
            "gdb -batch -ex 'set pagination off' -ex 'thread apply all bt {}' -ex detach -ex quit -p {}",
            sample_depth.min(2),
            pid
        ),
    )?;
    let validate_file = run_dir.join("validate-attach.txt");
    fs::write(&validate_file, &output.stdout)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "gdb could not attach to pid {pid}; check SYS_PTRACE/seccomp settings\n{}",
            stderr.trim()
        );
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !stdout
        .lines()
        .any(|line| line.starts_with("Thread") || line.starts_with("#0") || line.starts_with("#1"))
    {
        bail!("gdb attach succeeded but did not return a usable backtrace");
    }
    Ok(())
}

fn take_sample(
    ctx: &TaskContext,
    container: &str,
    pid: &str,
    sample_idx: usize,
    sample_depth: usize,
    raw_dir: &Path,
) -> Result<()> {
    let output = docker_exec_output(
        ctx,
        container,
        &format!(
            "gdb -batch -ex 'set pagination off' -ex 'thread apply all bt {}' -ex detach -ex quit -p {}",
            sample_depth, pid
        ),
    )?;
    let sample_file = raw_dir.join(format!("sample-{sample_idx:04}.txt"));
    let mut bytes = output.stdout;
    bytes.extend_from_slice(&output.stderr);
    fs::write(sample_file, bytes)?;
    Ok(())
}

fn summarize_samples(
    container: &str,
    duration_seconds: f64,
    interval_seconds: f64,
    sample_depth: usize,
    raw_dir: &Path,
) -> String {
    let mut counts = HashMap::<String, usize>::new();
    let mut files = fs::read_dir(raw_dir)
        .ok()
        .into_iter()
        .flat_map(|entries| entries.filter_map(Result::ok))
        .collect::<Vec<_>>();
    files.sort_by_key(|entry| entry.file_name());

    for entry in files {
        if let Ok(text) = fs::read_to_string(entry.path()) {
            for line in text.lines() {
                if !line.starts_with('#') {
                    continue;
                }
                let frame = trim_frame_prefix(line);
                if frame.is_empty() || is_runtime_frame(&frame) {
                    continue;
                }
                *counts.entry(frame).or_default() += 1;
            }
        }
    }

    let mut frames = counts.into_iter().collect::<Vec<_>>();
    frames.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));

    let mut summary = String::new();
    summary.push_str(&format!("Container: {container}\n"));
    summary.push_str(&format!("Duration: {duration_seconds}s\n"));
    summary.push_str(&format!("Interval: {interval_seconds}s\n"));
    summary.push_str(&format!("Sample depth: {sample_depth}\n\n"));
    summary.push_str("Top sampled application frames:\n");
    for (frame, count) in frames.into_iter().take(40) {
        summary.push_str(&format!("{count:7}  {frame}\n"));
    }
    summary.push_str(&format!("\nRaw samples: {}\n", raw_dir.display()));
    summary
}

fn trim_frame_prefix(line: &str) -> String {
    let trimmed = line.trim_start_matches('#');
    let trimmed = trimmed.trim_start_matches(|character: char| {
        character.is_ascii_digit() || character.is_whitespace()
    });
    trimmed.to_string()
}

fn is_runtime_frame(frame: &str) -> bool {
    let lower = frame.to_ascii_lowercase();
    [
        "futex",
        "epoll",
        "poll",
        "__lll_lock_wait",
        "clone",
        "start_thread",
        "__gi___",
        "libpthread",
        "libc.so",
        "linux-vdso",
        "ld-linux",
        "tokio::runtime::park",
        "std::thread::",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn docker_exec_checked(ctx: &TaskContext, container: &str, shell: &str) -> Result<()> {
    let mut command = ctx.command("docker");
    command.args(["exec", container, "sh", "-lc", shell]);
    crate::run_checked(&mut command)
}

fn docker_exec_capture(ctx: &TaskContext, container: &str, shell: &str) -> Result<String> {
    let mut command = ctx.command("docker");
    command.args(["exec", container, "sh", "-lc", shell]);
    crate::run_capture(&mut command)
}

fn docker_exec_output(
    ctx: &TaskContext,
    container: &str,
    shell: &str,
) -> Result<std::process::Output> {
    Ok(ctx
        .command("docker")
        .args(["exec", container, "sh", "-lc", shell])
        .output()?)
}
