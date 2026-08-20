use std::sync::Arc;

pub type SqliteWriterGate = Arc<tokio::sync::Mutex<()>>;

pub fn new_writer_gate() -> SqliteWriterGate {
    Arc::new(tokio::sync::Mutex::new(()))
}
