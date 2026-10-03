//! Logger behind the `log` macros: every line goes to the console straight
//! away, and is also kept in a RAM buffer for a log file.
//!
//! Writing to flash stalls the chip for up to ~18ms, so this never writes on
//! its own. Whoever owns the timing (the app) calls `flush_chunk()` when it's
//! safe. If the buffer fills first, new lines are dropped and counted, and a
//! marker records the gap in the file.
//!
//! Line format matches ESP-IDF's: `I (12345) target: message`.
//!
//! Files rotate the usual way: a new one per boot, `NNNN.txt`, numbered one
//! past the newest; a file that reaches MAX_FILE_BYTES continues in the next
//! number (its first line says so). The oldest are deleted to keep at most
//! MAX_FILES and the storage under MAX_USED_PERCENT full, and empty files
//! (boots that never wrote a line) are dropped. A boot loop once left 300+
//! files, and listing them ran the HTTP server out of memory.

use log::{Level, LevelFilter, Log, Metadata, Record};
use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// RAM held for lines not yet written: ~13s of busy radio traffic. Was 48KB;
/// cut to make room for WiFi (logs can be fetched over HTTP now)
const BUFFER_BYTES: usize = 16 * 1024;
/// Most written per `flush_chunk()`, cut at a line end
const CHUNK_BYTES: usize = 512;
/// Delete old log files until the partition is below this full
const MAX_USED_PERCENT: usize = 80;
/// A file this big continues in the next one: a walk (~500KB) is 2-3 files
const MAX_FILE_BYTES: u64 = 256 * 1024;
/// Most log files kept
const MAX_FILES: usize = 50;

/// Milliseconds since boot, for the line timestamps. Set by `init`
static UPTIME_MS: OnceLock<fn() -> u32> = OnceLock::new();

struct Logger {
    serial_level: LevelFilter,
    file_level: LevelFilter,
    buffer: Mutex<Buffer>,
    file: Mutex<Option<LogFile>>,
}

/// The file being written, and what rotating it needs
struct LogFile {
    file: File,
    dir: PathBuf,
    number: u32,
    bytes: u64,
    /// The storage's (used, total) bytes
    usage: fn() -> (usize, usize),
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

/// Start this boot's log file, `dir/NNNN.txt`, numbered one past the newest,
/// after dropping empty files and making room. `usage` gives the storage's
/// (used, total) bytes.
pub fn open_file(dir: &Path, usage: fn() -> (usize, usize)) -> std::io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let mut numbers = log_file_numbers(dir);
    numbers.sort_unstable();
    drop_empty_logs(dir, &mut numbers);
    let number = numbers.last().map_or(1, |newest| newest + 1);
    let log_file = LogFile::start(dir, number, usage)?;
    let path = log_path(dir, number);
    *LOGGER.file.lock().unwrap() = Some(log_file);
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
            let marker = format!("-- {} lines dropped (log buffer full) --\n", dropped);
            let _ = file.write(marker.as_bytes());
        }
        let _ = file.write(&chunk);
    }
    more
}

impl LogFile {
    /// Make room, then create file `number`
    fn start(dir: &Path, number: u32, usage: fn() -> (usize, usize)) -> std::io::Result<Self> {
        let mut numbers = log_file_numbers(dir);
        numbers.sort_unstable();
        make_room(dir, &mut numbers, usage);
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path(dir, number))?;
        Ok(LogFile {
            file,
            dir: dir.to_path_buf(),
            number,
            bytes: 0,
            usage,
        })
    }

    /// Append and sync. A file that has reached MAX_FILE_BYTES continues in
    /// the next number, starting with a line that says where it came from
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        if self.bytes >= MAX_FILE_BYTES {
            let previous = self.number;
            *self = LogFile::start(&self.dir, previous + 1, self.usage)?;
            let continued = format!("-- continued from {:04}.txt --\n", previous);
            self.file.write_all(continued.as_bytes())?;
            self.bytes += continued.len() as u64;
        }
        self.file.write_all(bytes)?;
        self.file.sync_all()?;
        self.bytes += bytes.len() as u64;
        Ok(())
    }
}

/// Delete the oldest files (`numbers` sorted, oldest first) until there's room
/// for one more under MAX_FILES, and the storage is under MAX_USED_PERCENT full
fn make_room(dir: &Path, numbers: &mut Vec<u32>, usage: fn() -> (usize, usize)) {
    while let Some(&oldest) = numbers.first() {
        let (used, total) = usage();
        let full = total > 0 && used * 100 / total >= MAX_USED_PERCENT;
        if numbers.len() < MAX_FILES && !full {
            break;
        }
        let _ = fs::remove_file(log_path(dir, oldest));
        numbers.remove(0);
    }
}

/// A boot that never wrote a line (it crashed or lost power first) leaves an
/// empty file; a boot loop leaves hundreds. Delete them
fn drop_empty_logs(dir: &Path, numbers: &mut Vec<u32>) {
    numbers.retain(|&number| {
        let path = log_path(dir, number);
        let empty = fs::metadata(&path).is_ok_and(|meta| meta.len() == 0);
        if empty {
            let _ = fs::remove_file(&path);
        }
        !empty
    });
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

/// The file for log NNNN: NNNN.txt
pub fn log_path(dir: &Path, number: u32) -> PathBuf {
    dir.join(format!("{:04}.txt", number))
}

/// Numbers of the existing NNNN.txt files
pub fn log_file_numbers(dir: &Path) -> Vec<u32> {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh directory for one test
    fn test_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("oswst-log-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sorted_numbers(dir: &Path) -> Vec<u32> {
        let mut numbers = log_file_numbers(dir);
        numbers.sort_unstable();
        numbers
    }

    fn plenty_of_space() -> (usize, usize) {
        (0, 100)
    }

    fn storage_full() -> (usize, usize) {
        (90, 100)
    }

    #[test]
    fn empty_files_are_dropped() {
        let dir = test_dir("empty");
        fs::write(log_path(&dir, 1), b"").unwrap();
        fs::write(log_path(&dir, 2), b"I (1) a: line\n").unwrap();
        fs::write(log_path(&dir, 3), b"").unwrap();
        let mut numbers = vec![1, 2, 3];
        drop_empty_logs(&dir, &mut numbers);
        assert_eq!(numbers, [2]);
        assert_eq!(sorted_numbers(&dir), [2]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn at_most_max_files_the_oldest_go() {
        let dir = test_dir("count");
        for n in 1..=60 {
            fs::write(log_path(&dir, n), b"x\n").unwrap();
        }
        LogFile::start(&dir, 61, plenty_of_space).unwrap();
        let numbers = sorted_numbers(&dir);
        assert_eq!(numbers.len(), MAX_FILES);
        assert_eq!(*numbers.first().unwrap(), 61 - MAX_FILES as u32 + 1);
        assert_eq!(*numbers.last().unwrap(), 61);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_full_storage_is_cleared_down_to_the_newest() {
        let dir = test_dir("space");
        for n in 1..=5 {
            fs::write(log_path(&dir, n), b"x\n").unwrap();
        }
        // usage() never drops below the cap here, so everything old goes
        LogFile::start(&dir, 6, storage_full).unwrap();
        assert_eq!(sorted_numbers(&dir), [6]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_full_file_continues_in_the_next() {
        let dir = test_dir("rotate");
        let mut log = LogFile::start(&dir, 7, plenty_of_space).unwrap();
        let chunk = vec![b'x'; 100 * 1024];
        // Three chunks: 300KB, past the cap, but a file is only checked
        // before a write, so 7 takes them all
        for _ in 0..3 {
            log.write(&chunk).unwrap();
        }
        assert_eq!(sorted_numbers(&dir), [7]);
        assert_eq!(fs::metadata(log_path(&dir, 7)).unwrap().len(), 300 * 1024);
        // The next write starts 8, which says where it came from
        log.write(&chunk).unwrap();
        assert_eq!(sorted_numbers(&dir), [7, 8]);
        let next = fs::read(log_path(&dir, 8)).unwrap();
        assert!(next.starts_with(b"-- continued from 0007.txt --\n"));
        log.write(&chunk).unwrap();
        assert_eq!(sorted_numbers(&dir), [7, 8]); // 8 isn't full yet
        fs::remove_dir_all(&dir).unwrap();
    }
}
