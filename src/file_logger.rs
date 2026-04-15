//! Dedicated file loggers for trades, performance, and supervisor events.
//!
//! Each logger appends JSON lines to its configured file path. Writes are
//! buffered through a channel so callers never block on I/O.

use std::path::Path;
use tokio::sync::mpsc;
use tracing::{error, info};

/// A non-blocking JSON-lines file logger.
#[derive(Clone)]
pub struct FileLogger {
    tx: mpsc::UnboundedSender<String>,
}

impl FileLogger {
    /// Spawn a background writer task for the given file path.
    /// Returns `None` if the path is empty or the file can't be created.
    pub fn new(path: &str) -> Option<Self> {
        if path.is_empty() {
            return None;
        }

        let file_path = Path::new(path);
        if let Some(dir) = file_path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }

        let file = match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(file_path)
        {
            Ok(f) => f,
            Err(e) => {
                error!("Failed to open log file {}: {}", path, e);
                return None;
            }
        };

        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        let path_owned = path.to_string();

        tokio::spawn(async move {
            use std::io::Write;
            let mut writer = std::io::BufWriter::new(file);
            while let Some(line) = rx.recv().await {
                if writeln!(writer, "{}", line).is_err() {
                    error!("Write failed for {}", path_owned);
                    break;
                }
                let _ = writer.flush();
            }
        });

        info!(path = path, "File logger started");
        Some(Self { tx })
    }

    /// Write a JSON-serializable value as a single line.
    pub fn log(&self, value: &impl serde::Serialize) {
        if let Ok(json) = serde_json::to_string(value) {
            let _ = self.tx.send(json);
        }
    }

}

/// No-op logger for when file logging is disabled.
#[derive(Clone)]
pub struct OptionalLogger(pub Option<FileLogger>);

impl OptionalLogger {
    pub fn log(&self, value: &impl serde::Serialize) {
        if let Some(ref l) = self.0 {
            l.log(value);
        }
    }
}
