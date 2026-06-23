use std::collections::VecDeque;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tokio::sync::broadcast;

const DEFAULT_CAPACITY: usize = 1000;
const BROADCAST_CAPACITY: usize = 256;
const MAX_LOG_FILE_BYTES: u64 = 10 * 1024 * 1024;
const MAX_ROTATED_LOG_FILES: usize = 5;

/// Thread-safe ring buffer that captures log lines.
#[derive(Clone)]
pub struct LogRingBuffer {
    inner: Arc<Mutex<RingBufferInner>>,
    tx: broadcast::Sender<String>,
}

struct RingBufferInner {
    lines: VecDeque<String>,
    capacity: usize,
    /// Accumulates partial writes (no trailing newline yet).
    partial: String,
}

impl LogRingBuffer {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        Self {
            inner: Arc::new(Mutex::new(RingBufferInner {
                lines: VecDeque::with_capacity(capacity),
                capacity,
                partial: String::new(),
            })),
            tx,
        }
    }

    pub fn with_default_capacity() -> Self {
        Self::new(DEFAULT_CAPACITY)
    }

    pub fn snapshot(&self, limit: usize) -> Vec<String> {
        let inner = self.inner.lock().unwrap();
        let safe_limit = limit.min(inner.lines.len());
        inner
            .lines
            .iter()
            .skip(inner.lines.len().saturating_sub(safe_limit))
            .cloned()
            .collect()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<String> {
        self.tx.subscribe()
    }
}

impl Write for LogRingBuffer {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let text = String::from_utf8_lossy(buf);
        let mut inner = self.inner.lock().unwrap();

        let mut new_lines = Vec::new();
        for ch in text.chars() {
            if ch == '\n' {
                if !inner.partial.is_empty() {
                    let line = std::mem::take(&mut inner.partial);
                    if inner.lines.len() >= inner.capacity {
                        inner.lines.pop_front();
                    }
                    inner.lines.push_back(line.clone());
                    new_lines.push(line);
                }
            } else {
                inner.partial.push(ch);
            }
        }
        drop(inner);
        for line in new_lines {
            let _ = self.tx.send(line);
        }

        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Adapter that lets `tracing_subscriber` write to our ring buffer.
/// Implements `tracing_subscriber::fmt::MakeWriter` by returning a clone
/// of the buffer (which implements `io::Write`).
#[derive(Clone)]
pub(crate) struct LogBufferWriter {
    buffer: LogRingBuffer,
}

impl LogBufferWriter {
    pub fn new(buffer: LogRingBuffer) -> Self {
        Self { buffer }
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogBufferWriter {
    type Writer = LogRingBuffer;

    fn make_writer(&'a self) -> Self::Writer {
        self.buffer.clone()
    }
}

pub(crate) fn open_log_file(path: &Path) -> io::Result<LogFileWriter> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    rotate_oversized_log_file(path, MAX_LOG_FILE_BYTES)?;
    let file = OpenOptions::new().create(true).append(true).open(path)?;
    Ok(LogFileWriter::new(file))
}

fn rotate_oversized_log_file(path: &Path, max_bytes: u64) -> io::Result<()> {
    rotate_oversized_log_file_with_compressor(path, max_bytes, compress_log_file_to_gzip)
}

fn rotate_oversized_log_file_with_compressor<F>(
    path: &Path,
    max_bytes: u64,
    compressor: F,
) -> io::Result<()>
where
    F: FnOnce(&Path, &Path) -> io::Result<()>,
{
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if metadata.len() <= max_bytes {
        return Ok(());
    }

    let temp_path = rotated_log_temp_path(path);
    remove_file_if_exists(&temp_path)?;
    if let Err(error) = compressor(path, &temp_path) {
        let _ = fs::remove_file(&temp_path);
        return Err(error);
    }

    if let Err(error) = shift_rotated_log_files(path) {
        let _ = fs::remove_file(&temp_path);
        return Err(error);
    }

    if let Err(error) = fs::rename(&temp_path, rotated_log_path(path, 1)) {
        let _ = fs::remove_file(&temp_path);
        return Err(error);
    }

    fs::remove_file(path)
}

fn compress_log_file_to_gzip(source: &Path, destination: &Path) -> io::Result<()> {
    let mut input = File::open(source)?;
    let output = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(destination)?;
    let mut encoder = flate2::write::GzEncoder::new(output, flate2::Compression::default());
    io::copy(&mut input, &mut encoder)?;
    let mut output = encoder.finish()?;
    output.flush()
}

fn shift_rotated_log_files(path: &Path) -> io::Result<()> {
    remove_file_if_exists(&rotated_log_path(path, MAX_ROTATED_LOG_FILES))?;
    for generation in (1..MAX_ROTATED_LOG_FILES).rev() {
        let source = rotated_log_path(path, generation);
        if !source.try_exists()? {
            continue;
        }
        fs::rename(source, rotated_log_path(path, generation + 1))?;
    }
    Ok(())
}

fn remove_file_if_exists(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn rotated_log_path(path: &Path, generation: usize) -> PathBuf {
    debug_assert!(generation > 0);
    let mut filename = path
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_else(|| OsString::from("scryer.log"));
    filename.push(format!(".{generation}.gz"));
    path.with_file_name(filename)
}

fn rotated_log_temp_path(path: &Path) -> PathBuf {
    let mut filename = path
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_else(|| OsString::from("scryer.log"));
    filename.push(".1.gz.tmp");
    path.with_file_name(filename)
}

#[derive(Clone)]
pub(crate) struct LogFileWriter {
    file: Arc<Mutex<File>>,
}

impl LogFileWriter {
    fn new(file: File) -> Self {
        Self {
            file: Arc::new(Mutex::new(file)),
        }
    }
}

pub(crate) struct LogFileWriteHandle {
    file: Arc<Mutex<File>>,
}

impl Write for LogFileWriteHandle {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.file.lock().unwrap().write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.lock().unwrap().flush()
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogFileWriter {
    type Writer = LogFileWriteHandle;

    fn make_writer(&'a self) -> Self::Writer {
        LogFileWriteHandle {
            file: self.file.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read as _;

    fn read_gzip(path: &Path) -> String {
        let file = File::open(path).expect("open gzip log");
        let mut decoder = flate2::read::GzDecoder::new(file);
        let mut contents = String::new();
        decoder
            .read_to_string(&mut contents)
            .expect("read gzip log");
        contents
    }

    fn write_gzip(path: &Path, contents: &str) {
        let file = File::create(path).expect("create gzip log");
        let mut encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        encoder
            .write_all(contents.as_bytes())
            .expect("write gzip log");
        encoder.finish().expect("finish gzip log");
    }

    #[test]
    fn open_log_file_creates_parent_directories_and_appends() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("nested").join("scryer.log");
        let writer = open_log_file(&path).expect("open log file");
        let mut handle = tracing_subscriber::fmt::MakeWriter::make_writer(&writer);

        writeln!(handle, "hello from scryer").expect("write log line");
        handle.flush().expect("flush log line");

        let contents = fs::read_to_string(path).expect("read log file");
        assert!(contents.contains("hello from scryer"));
    }

    #[test]
    fn oversized_log_file_rotates_to_compressed_dot_one() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("scryer.log");
        let rotated = dir.path().join("scryer.log.1.gz");
        let shifted = dir.path().join("scryer.log.2.gz");
        fs::write(&path, b"oversized").expect("seed active log");
        write_gzip(&rotated, "old rotated");

        rotate_oversized_log_file(&path, 4).expect("rotate oversized log");

        assert!(!path.exists());
        assert_eq!(read_gzip(&rotated), "oversized");
        assert_eq!(read_gzip(&shifted), "old rotated");
    }

    #[test]
    fn open_log_file_rotates_oversized_log_and_appends_to_fresh_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("scryer.log");
        let oversized = "x".repeat((MAX_LOG_FILE_BYTES + 1) as usize);
        fs::write(&path, oversized.as_bytes()).expect("seed active log");

        let writer = open_log_file(&path).expect("open log file");
        let mut handle = tracing_subscriber::fmt::MakeWriter::make_writer(&writer);
        writeln!(handle, "fresh line").expect("write fresh log line");
        handle.flush().expect("flush fresh log line");

        let rotated = read_gzip(&dir.path().join("scryer.log.1.gz"));
        assert_eq!(rotated.len(), oversized.len());
        assert!(rotated.chars().all(|ch| ch == 'x'));
        let active = fs::read_to_string(&path).expect("read fresh active log");
        assert!(active.contains("fresh line"));
        assert!(active.len() < oversized.len());
    }

    #[test]
    fn small_log_file_does_not_rotate() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("scryer.log");
        fs::write(&path, b"small").expect("seed active log");

        rotate_oversized_log_file(&path, 16).expect("skip small log");

        assert_eq!(fs::read_to_string(path).expect("read active log"), "small");
    }

    #[test]
    fn compressed_rotated_logs_shift_by_generation() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("scryer.log");
        fs::write(&path, b"current").expect("seed active log");
        write_gzip(&rotated_log_path(&path, 1), "one");
        write_gzip(&rotated_log_path(&path, 2), "two");
        write_gzip(&rotated_log_path(&path, 3), "three");

        rotate_oversized_log_file(&path, 4).expect("rotate oversized log");

        assert_eq!(read_gzip(&rotated_log_path(&path, 1)), "current");
        assert_eq!(read_gzip(&rotated_log_path(&path, 2)), "one");
        assert_eq!(read_gzip(&rotated_log_path(&path, 3)), "two");
        assert_eq!(read_gzip(&rotated_log_path(&path, 4)), "three");
    }

    #[test]
    fn compressed_rotated_logs_drop_oldest_generation() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("scryer.log");
        fs::write(&path, b"current").expect("seed active log");
        for generation in 1..=MAX_ROTATED_LOG_FILES {
            write_gzip(
                &rotated_log_path(&path, generation),
                &format!("old {generation}"),
            );
        }

        rotate_oversized_log_file(&path, 4).expect("rotate oversized log");

        assert_eq!(read_gzip(&rotated_log_path(&path, 1)), "current");
        assert_eq!(read_gzip(&rotated_log_path(&path, 2)), "old 1");
        assert_eq!(
            read_gzip(&rotated_log_path(&path, MAX_ROTATED_LOG_FILES)),
            "old 4"
        );
        assert!(!rotated_log_path(&path, MAX_ROTATED_LOG_FILES + 1).exists());
    }

    #[test]
    fn compression_failure_keeps_active_log_and_removes_partial_archive() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("scryer.log");
        fs::write(&path, b"oversized").expect("seed active log");

        let err = rotate_oversized_log_file_with_compressor(&path, 4, |_source, destination| {
            fs::write(destination, b"partial gzip").expect("seed partial archive");
            Err(io::Error::other("intentional compression failure"))
        })
        .expect_err("compression should fail");

        assert_eq!(err.kind(), io::ErrorKind::Other);
        assert_eq!(
            fs::read_to_string(&path).expect("read active log"),
            "oversized"
        );
        assert!(!rotated_log_path(&path, 1).exists());
        assert!(!rotated_log_temp_path(&path).exists());
    }
}
