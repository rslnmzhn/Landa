use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

pub const LOG_FILE_NAME: &str = "landa.log";
pub const DEFAULT_MAX_LOG_LINES: usize = 2000;
pub const DEFAULT_KEEP_LOG_LINES: usize = 1000;
pub const DEFAULT_MAX_LOG_BYTES: usize = 512 * 1024;
const MAX_PENDING_BYTES: usize = 64 * 1024;

static GLOBAL_LOG_SINK: OnceLock<Arc<RotatingLogSink>> = OnceLock::new();

fn global_sink() -> &'static Arc<RotatingLogSink> {
    GLOBAL_LOG_SINK.get_or_init(|| {
        Arc::new(RotatingLogSink::new(
            DEFAULT_MAX_LOG_LINES,
            DEFAULT_KEEP_LOG_LINES,
            DEFAULT_MAX_LOG_BYTES,
        ))
    })
}

pub fn init_tracing() {
    let _ = global_sink();
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    let stderr_layer = tracing_subscriber::fmt::layer().with_writer(std::io::stderr);
    let file_layer = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_writer(GlobalLogMakeWriter);

    let _ = tracing_subscriber::registry()
        .with(env_filter)
        .with(stderr_layer)
        .with(file_layer)
        .try_init();
}

pub fn attach_log_dir(app_data_dir: &Path) -> Result<PathBuf, String> {
    let log_path = app_data_dir.join(LOG_FILE_NAME);
    global_sink()
        .attach_path(log_path.clone())
        .map_err(|e| format!("Failed to initialize log file: {e}"))?;
    Ok(log_path)
}

pub fn ensure_log_file() -> Result<PathBuf, String> {
    global_sink().ensure_file()
}

pub fn read_log_tail(max_lines: usize) -> String {
    global_sink().read_tail(max_lines)
}

struct GlobalLogMakeWriter;

impl<'a> MakeWriter<'a> for GlobalLogMakeWriter {
    type Writer = BufferedLogWriter;

    fn make_writer(&'a self) -> Self::Writer {
        BufferedLogWriter {
            sink: Arc::clone(global_sink()),
            buf: Vec::with_capacity(256),
        }
    }
}

pub struct BufferedLogWriter {
    sink: Arc<RotatingLogSink>,
    buf: Vec<u8>,
}

impl Write for BufferedLogWriter {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.buf.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if !self.buf.is_empty() {
            self.sink.write_bytes(&self.buf);
            self.buf.clear();
        }
        Ok(())
    }
}

impl Drop for BufferedLogWriter {
    fn drop(&mut self) {
        let _ = self.flush();
    }
}

struct LogSinkInner {
    path: Option<PathBuf>,
    pending: Vec<u8>,
    max_lines: usize,
    keep_lines: usize,
    max_bytes: usize,
    current_lines: usize,
    current_bytes: usize,
}

pub struct RotatingLogSink {
    inner: Mutex<LogSinkInner>,
}

impl RotatingLogSink {
    pub fn new(max_lines: usize, keep_lines: usize, max_bytes: usize) -> Self {
        Self {
            inner: Mutex::new(LogSinkInner {
                path: None,
                pending: Vec::new(),
                max_lines: max_lines.max(1),
                keep_lines: keep_lines.max(1).min(max_lines.max(1)),
                max_bytes: max_bytes.max(64),
                current_lines: 0,
                current_bytes: 0,
            }),
        }
    }

    pub fn attach_path(&self, path: PathBuf) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut guard = self.inner.lock().expect("log sink lock poisoned");
        let existing = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(err) => return Err(err),
        };

        let mut combined = existing;
        if !guard.pending.is_empty() {
            combined.extend_from_slice(&guard.pending);
            guard.pending.clear();
        }

        let line_count = count_lines(&combined);
        let byte_count = combined.len();
        if line_count > guard.max_lines || byte_count > guard.max_bytes {
            let trimmed = trim_log_bytes(&combined, guard.keep_lines, guard.max_bytes);
            fs::write(&path, &trimmed)?;
            guard.current_lines = count_lines(&trimmed);
            guard.current_bytes = trimmed.len();
        } else {
            if !combined.is_empty() || !path.exists() {
                fs::write(&path, &combined)?;
            }
            guard.current_lines = line_count;
            guard.current_bytes = byte_count;
        }

        guard.path = Some(path);
        Ok(())
    }

    pub fn write_bytes(&self, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        let mut guard = self.inner.lock().expect("log sink lock poisoned");
        let Some(path) = guard.path.clone() else {
            let remaining = MAX_PENDING_BYTES.saturating_sub(guard.pending.len());
            if remaining > 0 {
                let take = data.len().min(remaining);
                guard.pending.extend_from_slice(&data[..take]);
            }
            return;
        };

        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
            if file.write_all(data).is_ok() {
                guard.current_bytes = guard.current_bytes.saturating_add(data.len());
                guard.current_lines = guard.current_lines.saturating_add(count_lines(data));
            }
        }

        if guard.current_lines > guard.max_lines || guard.current_bytes > guard.max_bytes {
            if let Ok(raw) = fs::read(&path) {
                let trimmed = trim_log_bytes(&raw, guard.keep_lines, guard.max_bytes);
                if fs::write(&path, &trimmed).is_ok() {
                    guard.current_lines = count_lines(&trimmed);
                    guard.current_bytes = trimmed.len();
                }
            }
        }
    }

    pub fn ensure_file(&self) -> Result<PathBuf, String> {
        let guard = self.inner.lock().expect("log sink lock poisoned");
        let path = guard
            .path
            .clone()
            .ok_or_else(|| "Log file path is not initialized".to_string())?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create log directory: {e}"))?;
        }
        if !path.exists() {
            fs::write(&path, b"").map_err(|e| format!("Failed to create log file: {e}"))?;
        }
        Ok(path)
    }

    pub fn read_tail(&self, max_lines: usize) -> String {
        let guard = self.inner.lock().expect("log sink lock poisoned");
        let Some(path) = guard.path.as_ref() else {
            return String::new();
        };
        let Ok(raw) = fs::read(path) else {
            return String::new();
        };
        let text = String::from_utf8_lossy(&raw);
        let lines: Vec<&str> = text.lines().collect();
        let start = lines.len().saturating_sub(max_lines);
        lines[start..].join("\n")
    }
}

fn count_lines(bytes: &[u8]) -> usize {
    bytes.iter().filter(|&&b| b == b'\n').count()
}

fn trim_log_bytes(raw: &[u8], keep_lines: usize, max_bytes: usize) -> Vec<u8> {
    let text = String::from_utf8_lossy(raw);
    let mut lines: Vec<&str> = text.lines().collect();
    if lines.len() > keep_lines {
        let start = lines.len() - keep_lines;
        lines = lines.split_off(start);
    }

    let mut start_idx = 0usize;
    let mut total_bytes: usize = lines.iter().map(|l| l.len() + 1).sum();
    while total_bytes > max_bytes && start_idx + 1 < lines.len() {
        total_bytes = total_bytes.saturating_sub(lines[start_idx].len() + 1);
        start_idx += 1;
    }

    let mut out = lines[start_idx..].join("\n");
    if !out.is_empty() {
        out.push('\n');
    }

    if out.len() > max_bytes {
        let keep_from = out.len() - max_bytes;
        let mut boundary = keep_from;
        while boundary < out.len() && !out.is_char_boundary(boundary) {
            boundary += 1;
        }
        out = out[boundary..].to_string();
    }

    out.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn rotates_when_line_count_exceeds_threshold() {
        let temp_dir = std::env::temp_dir().join(format!("landa-log-lines-{}", Uuid::new_v4()));
        let log_path = temp_dir.join(LOG_FILE_NAME);
        let sink = RotatingLogSink::new(10, 5, 1024 * 1024);
        sink.attach_path(log_path.clone()).unwrap();

        for i in 1..=11 {
            sink.write_bytes(format!("line-{i}\n").as_bytes());
        }

        let content_after_rotate = fs::read_to_string(&log_path).unwrap();
        let rotated_lines: Vec<&str> = content_after_rotate.lines().collect();
        assert_eq!(
            rotated_lines,
            vec!["line-7", "line-8", "line-9", "line-10", "line-11"]
        );

        for i in 12..=15 {
            sink.write_bytes(format!("line-{i}\n").as_bytes());
        }

        let content = fs::read_to_string(&log_path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert!(
            lines.len() <= 10,
            "expected <= 10 lines, got {}",
            lines.len()
        );
        assert!(!lines.contains(&"line-1"));
        assert!(lines.contains(&"line-15"));
        assert_eq!(sink.read_tail(3), "line-13\nline-14\nline-15");

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn rotates_when_byte_size_exceeds_threshold() {
        let temp_dir = std::env::temp_dir().join(format!("landa-log-bytes-{}", Uuid::new_v4()));
        let log_path = temp_dir.join(LOG_FILE_NAME);
        let sink = RotatingLogSink::new(10_000, 5_000, 256);
        sink.attach_path(log_path.clone()).unwrap();

        for i in 1..=12 {
            let line = format!("entry-{i:02}-xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\n");
            sink.write_bytes(line.as_bytes());
        }

        let raw = fs::read(&log_path).unwrap();
        assert!(
            raw.len() <= 256,
            "expected log size <= 256 bytes, got {}",
            raw.len()
        );
        let content = String::from_utf8_lossy(&raw);
        assert!(!content.contains("entry-01-"));
        assert!(content.contains("entry-12-"));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn attach_path_flushes_pending_logs_and_rotates_existing_file() {
        let temp_dir = std::env::temp_dir().join(format!("landa-log-attach-{}", Uuid::new_v4()));
        fs::create_dir_all(&temp_dir).unwrap();
        let log_path = temp_dir.join(LOG_FILE_NAME);
        fs::write(&log_path, "old-1\nold-2\nold-3\nold-4\nold-5\n").unwrap();

        let sink = RotatingLogSink::new(6, 3, 1024 * 1024);
        sink.write_bytes(b"early-1\nearly-2\n");
        sink.attach_path(log_path.clone()).unwrap();

        let content = fs::read_to_string(&log_path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines, vec!["old-5", "early-1", "early-2"]);

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
