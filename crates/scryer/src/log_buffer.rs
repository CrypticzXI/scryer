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
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if metadata.len() <= max_bytes {
        return Ok(());
    }

    let rotated = rotated_log_path(path);
    match fs::remove_file(&rotated) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    fs::rename(path, rotated)
}

fn rotated_log_path(path: &Path) -> PathBuf {
    let mut filename = path
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_else(|| OsString::from("scryer.log"));
    filename.push(".1");
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
    fn oversized_log_file_rotates_to_dot_one() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("scryer.log");
        let rotated = dir.path().join("scryer.log.1");
        fs::write(&path, b"oversized").expect("seed active log");
        fs::write(&rotated, b"old rotated").expect("seed rotated log");

        rotate_oversized_log_file(&path, 4).expect("rotate oversized log");

        assert!(!path.exists());
        assert_eq!(
            fs::read_to_string(rotated).expect("read rotated log"),
            "oversized"
        );
    }

    #[test]
    fn small_log_file_does_not_rotate() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("scryer.log");
        fs::write(&path, b"small").expect("seed active log");

        rotate_oversized_log_file(&path, 16).expect("skip small log");

        assert_eq!(fs::read_to_string(path).expect("read active log"), "small");
    }
}
