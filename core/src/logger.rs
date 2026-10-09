//! Logger behind the `log` macros: every line goes to the console straight
//! away, and is also kept in a RAM buffer for a log file.
//!
//! Writing to flash stalls the chip for up to ~18ms, so this never writes on
//! its own. Whoever owns the timing (the app) calls `flush_chunk()` when it's
//! safe. If the buffer fills first, new lines are dropped and counted, and a
//! marker records the gap in the file.
//!
//! The buffer's memory comes from the caller (`init`), so the firmware can
//! put it in PSRAM, away from the internal heap.
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
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

/// Most written per `flush_chunk()`, cut at a line end
const CHUNK_BYTES: usize = 512;
/// Delete old log files until the partition is below this full. The rest
/// is kept for an update bundle's download (bundle.rs, ~1.4MB of ~10MB)
const MAX_USED_PERCENT: usize = 60;
/// A file this big continues in the next one. An hour's walk logs ~500KB
/// (2026-10-04): 1MB keeps one in a single file. MAX_USED_PERCENT still
/// caps the total (~6 files this size on the 10MB partition)
const MAX_FILE_BYTES: u64 = 1024 * 1024;
/// Most log files kept
const MAX_FILES: usize = 50;

/// Milliseconds since boot, for the line timestamps. Set by `init`
static UPTIME_MS: OnceLock<fn() -> u32> = OnceLock::new();

/// Where log files go, and the storage's (used, total) bytes: set at boot
/// (`set_storage`) whether logging to flash is on or not, so it can be
/// switched on later
static STORAGE: Mutex<Option<(PathBuf, Usage)>> = Mutex::new(None);

/// The storage's (used, total) bytes
type Usage = fn() -> (usize, usize);

/// Whether lines are kept for a file: from boot until it's decided (the
/// settings load), then while one is being written. Off, a line goes to the
/// serial port only, so nothing logged while logging is off ever reaches
/// the flash, even if it's switched on later
static KEEPING: AtomicBool = AtomicBool::new(true);

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
    lines: Ring,
    dropped: u32, // lines lost since the last flush because the buffer was full
}

/// Bytes waiting to be written, in a fixed piece of memory used round and
/// round: the oldest byte is at `start`, and the `len` after it (wrapping
/// past the end) are waiting
struct Ring {
    memory: &'static mut [u8],
    start: usize,
    len: usize,
}

static LOGGER: Logger = Logger {
    serial_level: LevelFilter::Info,
    file_level: LevelFilter::Info,
    buffer: Mutex::new(Buffer {
        lines: Ring {
            memory: &mut [],
            start: 0,
            len: 0,
        },
        dropped: 0,
    }),
    file: Mutex::new(None),
};

/// Install as the `log` backend. The console works from here on; the file
/// only once `open_file()` succeeds. `uptime_ms` stamps each line, and
/// `memory` holds the lines waiting for the file.
pub fn init(uptime_ms: fn() -> u32, memory: &'static mut [u8]) {
    let _ = UPTIME_MS.set(uptime_ms);
    LOGGER.buffer.lock().unwrap().lines.memory = memory;
    log::set_logger(&LOGGER).unwrap();
    log::set_max_level(LOGGER.serial_level.max(LOGGER.file_level));
}

/// Where log files go: `dir`, on a storage whose (used, total) bytes
/// `usage` gives. Nothing is written until `start_file()`
pub fn set_storage(dir: &Path, usage: fn() -> (usize, usize)) {
    *STORAGE.lock().unwrap() = Some((dir.to_path_buf(), usage));
}

/// Start writing a log file, `NNNN.txt`, numbered one past the newest,
/// after dropping empty files and making room: at boot if log_to_flash is
/// on, or when it's switched on. The lines waiting in RAM go to it. One
/// already open stays as it is
pub fn start_file() -> std::io::Result<PathBuf> {
    let (dir, usage) = storage()?;
    let mut file = LOGGER.file.lock().unwrap();
    if let Some(open) = file.as_ref() {
        return Ok(log_path(&open.dir, open.number));
    }
    fs::create_dir_all(&dir)?;
    let mut numbers = log_file_numbers(&dir);
    numbers.sort_unstable();
    drop_empty_logs(&dir, &mut numbers);
    let number = numbers.last().map_or(1, |newest| newest + 1);
    *file = Some(LogFile::start(&dir, number, usage)?);
    KEEPING.store(true, Ordering::Relaxed);
    Ok(log_path(&dir, number))
}

/// Stop writing to flash: log_to_flash off (at boot, or switched off). The
/// file is closed, the lines waiting for it are dropped (they must never
/// get there), and no more are kept until `start_file()`
pub fn stop_file() {
    KEEPING.store(false, Ordering::Relaxed);
    LOGGER.file.lock().unwrap().take();
    discard_pending();
}

fn storage() -> std::io::Result<(PathBuf, Usage)> {
    STORAGE
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no storage"))
}

/// Is anything waiting to be written?
pub fn pending() -> bool {
    let buffer = LOGGER.buffer.lock().unwrap();
    buffer.lines.len > 0 || buffer.dropped > 0
}

/// Write up to one chunk of buffered lines to the file and sync it. Stalls
/// the chip for up to ~18ms, so only call it when timing doesn't matter.
/// Returns whether more is still waiting.
pub fn flush_chunk() -> bool {
    let (chunk, dropped, more) = {
        let mut buffer = LOGGER.buffer.lock().unwrap();
        let chunk = buffer.lines.take_chunk(CHUNK_BYTES);
        let dropped = std::mem::take(&mut buffer.dropped);
        (chunk, dropped, buffer.lines.len > 0)
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

impl Ring {
    /// Append `bytes` whole, or not at all if they don't fit
    fn push(&mut self, bytes: &[u8]) -> bool {
        let capacity = self.memory.len();
        if self.len + bytes.len() > capacity {
            return false;
        }
        // The free space starts just past the last waiting byte
        let mut at = (self.start + self.len) % capacity;
        for &byte in bytes {
            self.memory[at] = byte;
            at = (at + 1) % capacity;
        }
        self.len += bytes.len();
        true
    }

    /// Remove and return up to `limit` of the oldest bytes, cut after the last
    /// full line among them, if there is one
    fn take_chunk(&mut self, limit: usize) -> Vec<u8> {
        let capacity = self.memory.len();
        let limit = self.len.min(limit);
        let mut chunk = Vec::with_capacity(limit);
        for offset in 0..limit {
            chunk.push(self.memory[(self.start + offset) % capacity]);
        }
        let cut = chunk
            .iter()
            .rposition(|&byte| byte == b'\n')
            .map_or(limit, |newline| newline + 1);
        chunk.truncate(cut);
        if cut > 0 {
            self.start = (self.start + cut) % capacity;
            self.len -= cut;
        }
        chunk
    }
}

/// Delete every log file, and the lines not written yet: the menu's
/// Privacy > Erase logs. A file being written is closed first, and a fresh
/// one (0001.txt) started after, so logging carries on as it was. Returns
/// how many files went
pub fn erase_all() -> std::io::Result<usize> {
    let (dir, usage) = storage()?;
    let mut file = LOGGER.file.lock().unwrap();
    // Closed (dropped) before its file is deleted
    let was_open = file.take().is_some();
    let mut erased = 0;
    for number in log_file_numbers(&dir) {
        fs::remove_file(log_path(&dir, number))?;
        erased += 1;
    }
    discard_pending();
    if was_open {
        *file = Some(LogFile::start(&dir, 1, usage)?);
    }
    Ok(erased)
}

/// Forget the lines not written yet: log_to_flash switched off (they were
/// meant for the file, and must never reach it), or Erase logs
fn discard_pending() {
    let mut buffer = LOGGER.buffer.lock().unwrap();
    buffer.lines.start = 0;
    buffer.lines.len = 0;
    buffer.dropped = 0;
}

/// Write everything buffered to the file, however long that stalls: for
/// just before a reboot, which would lose it (firmware.rs reboot_soon)
pub fn flush_all() {
    while flush_chunk() {}
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
        if record.level() <= self.file_level && KEEPING.load(Ordering::Relaxed) {
            let mut buffer = self.buffer.lock().unwrap();
            if !buffer.lines.push(line.as_bytes()) {
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

    /// A ring over a fresh piece of memory, leaked like the firmware's
    fn ring(capacity: usize) -> Ring {
        Ring {
            memory: Box::leak(vec![0u8; capacity].into_boxed_slice()),
            start: 0,
            len: 0,
        }
    }

    #[test]
    fn a_line_that_does_not_fit_is_refused_whole() {
        let mut lines = ring(10);
        assert!(lines.push(b"12345\n"));
        assert!(!lines.push(b"12345\n"));
        assert_eq!(lines.len, 6);
    }

    #[test]
    fn chunks_end_at_a_line_end() {
        let mut lines = ring(64);
        lines.push(b"one\n");
        lines.push(b"two\n");
        lines.push(b"three\n");
        // 10 bytes reach into "three": the chunk stops after "two"
        assert_eq!(lines.take_chunk(10), b"one\ntwo\n");
        assert_eq!(lines.take_chunk(10), b"three\n");
        assert_eq!(lines.len, 0);
    }

    #[test]
    fn a_line_longer_than_a_chunk_is_split() {
        let mut lines = ring(64);
        lines.push(b"abcdefgh\n");
        assert_eq!(lines.take_chunk(4), b"abcd");
        assert_eq!(lines.take_chunk(8), b"efgh\n");
    }

    #[test]
    fn lines_wrap_past_the_end_of_memory() {
        let mut lines = ring(8);
        lines.push(b"aaaaa\n");
        assert_eq!(lines.take_chunk(8), b"aaaaa\n");
        // Starts at 6 of 8: wraps round to the front
        assert!(lines.push(b"bbbbbb\n"));
        assert_eq!(lines.take_chunk(8), b"bbbbbb\n");
    }

    fn plenty_of_space() -> (usize, usize) {
        (0, 100)
    }

    fn storage_full() -> (usize, usize) {
        (90, 100)
    }

    #[test]
    fn erasing_deletes_every_log() {
        let dir = test_dir("erase");
        for n in 1..=3 {
            fs::write(log_path(&dir, n), b"I (1) a: line\n").unwrap();
        }
        fs::write(dir.join("notes.md"), b"not a log").unwrap();
        set_storage(&dir, plenty_of_space);
        assert_eq!(erase_all().unwrap(), 3);
        assert_eq!(sorted_numbers(&dir), Vec::<u32>::new());
        assert!(dir.join("notes.md").exists()); // only log files go
        fs::remove_dir_all(&dir).unwrap();
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
        // Each 2/5 of a file: three go past the cap, but a file is only
        // checked before a write, so 7 takes them all
        let chunk = vec![b'x'; (MAX_FILE_BYTES * 2 / 5) as usize];
        for _ in 0..3 {
            log.write(&chunk).unwrap();
        }
        assert_eq!(sorted_numbers(&dir), [7]);
        assert_eq!(
            fs::metadata(log_path(&dir, 7)).unwrap().len(),
            3 * chunk.len() as u64
        );
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
