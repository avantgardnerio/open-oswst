//! Logger behind the `log` macros: every line goes to the console straight
//! away, and is also kept in a RAM buffer for a log file.
//!
//! Writing to flash stalls the chip for up to ~18ms, so this never writes on
//! its own. Whoever owns the timing (the app) calls `flush_chunk()` when it's
//! safe. If the buffer fills first, new lines are dropped and counted, and a
//! marker records the gap in the file.
//!
//! Line format matches ESP-IDF's: `I (12345) target: message`.

use log::{Level, LevelFilter, Log, Metadata, Record};
use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// RAM held for lines not yet written: ~40s of busy radio traffic
const BUFFER_BYTES: usize = 48 * 1024;
/// Most written per `flush_chunk()`, cut at a line end
const CHUNK_BYTES: usize = 512;
/// Delete old log files until the partition is below this full
const MAX_USED_PERCENT: usize = 80;

/// Milliseconds since boot, for the line timestamps. Set by `init`
static UPTIME_MS: OnceLock<fn() -> u32> = OnceLock::new();

struct Logger {
    serial_level: LevelFilter,
    file_level: LevelFilter,
    buffer: Mutex<Buffer>,
    file: Mutex<Option<File>>,
}

struct Buffer {
    bytes: VecDeque<u8>,
    dropped: u32, // lines lost since the last flush because the buffer was full
}

static LOGGER: Logger = Logger {
    serial_level: LevelFilter::Info,
    file_level: LevelFilter::Info,
    buffer: Mutex::new(Buffer {
        bytes: VecDeque::new(),
        dropped: 0,
    }),
    file: Mutex::new(None),
};

/// Install as the `log` backend. The console works from here on; the file
/// only once `open_file()` succeeds. `uptime_ms` stamps each line.
pub fn init(uptime_ms: fn() -> u32) {
    let _ = UPTIME_MS.set(uptime_ms);
    LOGGER
        .buffer
        .lock()
        .unwrap()
        .bytes
        .reserve_exact(BUFFER_BYTES);
    log::set_logger(&LOGGER).unwrap();
    log::set_max_level(LOGGER.serial_level.max(LOGGER.file_level));
}

/// Start this boot's log file, `dir/NNNN.txt`, numbered one past the newest.
/// Clears out the oldest files first while `usage()` (used, total bytes) says
/// the storage is filling up.
pub fn open_file(dir: &Path, usage: impl Fn() -> (usize, usize)) -> std::io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let mut numbers = log_file_numbers(dir);
    numbers.sort_unstable();
    for oldest in numbers.clone() {
        let (used, total) = usage();
        if total == 0 || used * 100 / total < MAX_USED_PERCENT {
            break;
        }
        let _ = fs::remove_file(log_path(dir, oldest));
        numbers.retain(|&n| n != oldest);
    }
    let path = log_path(dir, numbers.last().map_or(1, |newest| newest + 1));

    let file = OpenOptions::new().create(true).append(true).open(&path)?;
    *LOGGER.file.lock().unwrap() = Some(file);
    Ok(path)
}

/// Is anything waiting to be written?
pub fn pending() -> bool {
    let buffer = LOGGER.buffer.lock().unwrap();
    !buffer.bytes.is_empty() || buffer.dropped > 0
}

/// Write up to one chunk of buffered lines to the file and sync it. Stalls
/// the chip for up to ~18ms, so only call it when timing doesn't matter.
/// Returns whether more is still waiting.
pub fn flush_chunk() -> bool {
    let (chunk, dropped, more) = {
        let mut buffer = LOGGER.buffer.lock().unwrap();
        let limit = buffer.bytes.len().min(CHUNK_BYTES);
        // Cut after the last full line in the chunk, if there is one
        let cut = buffer
            .bytes
            .range(..limit)
            .rposition(|&b| b == b'\n')
            .map_or(limit, |i| i + 1);
        let chunk: Vec<u8> = buffer.bytes.drain(..cut).collect();
        let dropped = std::mem::take(&mut buffer.dropped);
        (chunk, dropped, !buffer.bytes.is_empty())
    };

    // Buffer lock released: logging carries on while the flash write runs
    let mut file = LOGGER.file.lock().unwrap();
    if let Some(file) = file.as_mut() {
        if dropped > 0 {
            let _ = writeln!(file, "-- {} lines dropped (log buffer full) --", dropped);
        }
        let _ = file.write_all(&chunk);
        let _ = file.sync_all();
    }
    more
}

impl Log for Logger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= self.serial_level || metadata.level() <= self.file_level
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let line = format!(
            "{} ({}) {}: {}\n",
            letter(record.level()),
            UPTIME_MS.get().map_or(0, |uptime_ms| uptime_ms()),
            record.target(),
            record.args()
        );

        if record.level() <= self.serial_level {
            // Not print!: with no USB host attached (on battery), the console
            // write fails, and print! panics on failure. Drop the line instead.
            let _ = std::io::stdout().write_all(line.as_bytes());
        }
        if record.level() <= self.file_level {
            let mut buffer = self.buffer.lock().unwrap();
            if buffer.bytes.len() + line.len() <= BUFFER_BYTES {
                buffer.bytes.extend(line.as_bytes());
            } else {
                buffer.dropped += 1;
            }
        }
    }

    fn flush(&self) {}
}

fn letter(level: Level) -> char {
    match level {
        Level::Error => 'E',
        Level::Warn => 'W',
        Level::Info => 'I',
        Level::Debug => 'D',
        Level::Trace => 'V',
    }
}

fn log_path(dir: &Path, number: u32) -> PathBuf {
    dir.join(format!("{:04}.txt", number))
}

/// Numbers of the existing NNNN.txt files
fn log_file_numbers(dir: &Path) -> Vec<u32> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| {
            let name = entry.ok()?.file_name().into_string().ok()?;
            name.strip_suffix(".txt")?.parse().ok()
        })
        .collect()
}
