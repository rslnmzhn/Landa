use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, OnceLock};

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

fn parse_rust_log_level_str(raw: &str) -> tracing_subscriber::filter::LevelFilter {
    let token = raw
        .split(',')
        .next()
        .unwrap_or("")
        .rsplit('=')
        .next()
        .unwrap_or("")
        .trim();
    match token.to_ascii_lowercase().as_str() {
        "off" => tracing_subscriber::filter::LevelFilter::OFF,
        "error" => tracing_subscriber::filter::LevelFilter::ERROR,
        "warn" => tracing_subscriber::filter::LevelFilter::WARN,
        "debug" => tracing_subscriber::filter::LevelFilter::DEBUG,
        "trace" => tracing_subscriber::filter::LevelFilter::TRACE,
        _ => tracing_subscriber::filter::LevelFilter::INFO,
    }
}

fn parse_rust_log_level() -> tracing_subscriber::filter::LevelFilter {
    std::env::var("RUST_LOG")
        .map(|raw| parse_rust_log_level_str(&raw))
        .unwrap_or(tracing_subscriber::filter::LevelFilter::INFO)
}

pub fn init_tracing() {
    let _ = global_sink();
    let level_filter = parse_rust_log_level();

    let stderr_layer = tracing_subscriber::fmt::layer().with_writer(std::io::stderr);
    let file_layer = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_writer(GlobalLogMakeWriter);

    let _ = tracing_subscriber::registry()
        .with(level_filter)
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

enum LogCommand {
    Attach {
        path: PathBuf,
        reply: mpsc::SyncSender<std::io::Result<()>>,
    },
    Write(Vec<u8>),
    EnsureFile {
        reply: mpsc::SyncSender<Result<PathBuf, String>>,
    },
    ReadTail {
        max_lines: usize,
        reply: mpsc::SyncSender<String>,
    },
    #[cfg(test)]
    Flush(mpsc::SyncSender<()>),
}

struct LogWorker {
    path: Option<PathBuf>,
    writer: Option<BufWriter<File>>,
    pending: Vec<u8>,
    max_lines: usize,
    keep_lines: usize,
    max_bytes: usize,
    current_lines: usize,
    current_bytes: usize,
}

impl LogWorker {
    fn new(max_lines: usize, keep_lines: usize, max_bytes: usize) -> Self {
        Self {
            path: None,
            writer: None,
            pending: Vec::new(),
            max_lines: max_lines.max(1),
            keep_lines: keep_lines.max(1).min(max_lines.max(1)),
            max_bytes: max_bytes.max(64),
            current_lines: 0,
            current_bytes: 0,
        }
    }

    fn open_append_writer(path: &Path) -> std::io::Result<BufWriter<File>> {
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(BufWriter::new(file))
    }

    fn flush_writer(&mut self) {
        if let Some(writer) = self.writer.as_mut() {
            let _ = writer.flush();
        }
    }

    fn attach_path(&mut self, path: PathBuf) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        self.flush_writer();
        self.writer = None;

        let existing = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(err) => return Err(err),
        };

        let mut combined = existing;
        if !self.pending.is_empty() {
            combined.extend_from_slice(&self.pending);
            self.pending.clear();
        }

        let line_count = count_lines(&combined);
        let byte_count = combined.len();
        if line_count > self.max_lines || byte_count > self.max_bytes {
            let trimmed = trim_log_bytes(&combined, self.keep_lines, self.max_bytes);
            fs::write(&path, &trimmed)?;
            self.current_lines = count_lines(&trimmed);
            self.current_bytes = trimmed.len();
        } else {
            if !combined.is_empty() || !path.exists() {
                fs::write(&path, &combined)?;
            }
            self.current_lines = line_count;
            self.current_bytes = byte_count;
        }

        self.writer = Some(Self::open_append_writer(&path)?);
        self.path = Some(path);
        Ok(())
    }

    fn write_bytes(&mut self, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        let Some(path) = self.path.clone() else {
            let remaining = MAX_PENDING_BYTES.saturating_sub(self.pending.len());
            if remaining > 0 {
                let take = data.len().min(remaining);
                self.pending.extend_from_slice(&data[..take]);
            }
            return;
        };

        if self.writer.is_none() {
            self.writer = Self::open_append_writer(&path).ok();
        }

        if let Some(writer) = self.writer.as_mut() {
            if writer.write_all(data).is_ok() {
                self.current_bytes = self.current_bytes.saturating_add(data.len());
                self.current_lines = self.current_lines.saturating_add(count_lines(data));
            }
        }

        if self.current_lines > self.max_lines || self.current_bytes > self.max_bytes {
            self.flush_writer();
            self.writer = None;
            if let Ok(raw) = fs::read(&path) {
                let trimmed = trim_log_bytes(&raw, self.keep_lines, self.max_bytes);
                if fs::write(&path, &trimmed).is_ok() {
                    self.current_lines = count_lines(&trimmed);
                    self.current_bytes = trimmed.len();
                }
            }
            self.writer = Self::open_append_writer(&path).ok();
        }
    }

    fn ensure_file(&mut self) -> Result<PathBuf, String> {
        self.flush_writer();
        let path = self
            .path
            .clone()
            .ok_or_else(|| "Log file path is not initialized".to_string())?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create log directory: {e}"))?;
        }
        if !path.exists() {
            fs::write(&path, b"").map_err(|e| format!("Failed to create log file: {e}"))?;
            self.writer = Self::open_append_writer(&path).ok();
        }
        Ok(path)
    }

    fn read_tail(&mut self, max_lines: usize) -> String {
        self.flush_writer();
        let Some(path) = self.path.as_ref() else {
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

    fn handle(&mut self, cmd: LogCommand) {
        match cmd {
            LogCommand::Attach { path, reply } => {
                let _ = reply.send(self.attach_path(path));
            }
            LogCommand::Write(data) => {
                self.write_bytes(&data);
            }
            LogCommand::EnsureFile { reply } => {
                let _ = reply.send(self.ensure_file());
            }
            LogCommand::ReadTail { max_lines, reply } => {
                let _ = reply.send(self.read_tail(max_lines));
            }
            #[cfg(test)]
            LogCommand::Flush(reply) => {
                self.flush_writer();
                let _ = reply.send(());
            }
        }
    }
}

pub struct RotatingLogSink {
    tx: mpsc::Sender<LogCommand>,
}

impl RotatingLogSink {
    pub fn new(max_lines: usize, keep_lines: usize, max_bytes: usize) -> Self {
        let (tx, rx) = mpsc::channel::<LogCommand>();
        let mut worker = LogWorker::new(max_lines, keep_lines, max_bytes);
        let _ = std::thread::Builder::new()
            .name("landa-log-writer".to_string())
            .spawn(move || {
                while let Ok(cmd) = rx.recv() {
                    worker.handle(cmd);
                    while let Ok(next) = rx.try_recv() {
                        worker.handle(next);
                    }
                    worker.flush_writer();
                }
            });
        Self { tx }
    }

    pub fn attach_path(&self, path: PathBuf) -> std::io::Result<()> {
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        self.tx
            .send(LogCommand::Attach {
                path,
                reply: reply_tx,
            })
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "log worker stopped"))?;
        reply_rx
            .recv()
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "log worker stopped"))?
    }

    pub fn write_bytes(&self, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        let _ = self.tx.send(LogCommand::Write(data.to_vec()));
    }

    pub fn ensure_file(&self) -> Result<PathBuf, String> {
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        self.tx
            .send(LogCommand::EnsureFile { reply: reply_tx })
            .map_err(|_| "Log worker stopped".to_string())?;
        reply_rx
            .recv()
            .map_err(|_| "Log worker stopped".to_string())?
    }

    pub fn read_tail(&self, max_lines: usize) -> String {
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        if self
            .tx
            .send(LogCommand::ReadTail {
                max_lines,
                reply: reply_tx,
            })
            .is_err()
        {
            return String::new();
        }
        reply_rx.recv().unwrap_or_default()
    }

    #[cfg(test)]
    fn flush(&self) {
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        if self.tx.send(LogCommand::Flush(reply_tx)).is_ok() {
            let _ = reply_rx.recv();
        }
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
        sink.flush();

        let content_after_rotate = fs::read_to_string(&log_path).unwrap();
        let rotated_lines: Vec<&str> = content_after_rotate.lines().collect();
        assert_eq!(
            rotated_lines,
            vec!["line-7", "line-8", "line-9", "line-10", "line-11"]
        );

        for i in 12..=15 {
            sink.write_bytes(format!("line-{i}\n").as_bytes());
        }
        sink.flush();

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
        sink.flush();

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

    #[test]
    fn parses_rust_log_level_directives() {
        use tracing_subscriber::filter::LevelFilter;
        assert_eq!(parse_rust_log_level_str("debug"), LevelFilter::DEBUG);
        assert_eq!(
            parse_rust_log_level_str("landa_core=warn,hyper=error"),
            LevelFilter::WARN
        );
        assert_eq!(parse_rust_log_level_str("OFF"), LevelFilter::OFF);
        assert_eq!(parse_rust_log_level_str("unknown"), LevelFilter::INFO);
    }
}
