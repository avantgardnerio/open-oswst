//! Reading a .gz file as a stream: the gzip header, then deflate data,
//! inflated into a 32KB window as it's read. The inflating itself is one
//! step at a time through `InflateStep`: on the radio miniz's
//! tinfl_decompress, which the ESP32-S3 has in ROM (no flash spent); in the
//! tests miniz_oxide, the Rust port of the same code.
//!
//! The gzip trailer (CRC-32, length) isn't checked: the whole file's SHA-256
//! is, before it's read (update bundles, bundle.rs).

use std::io::{self, Read};

/// Deflate's longest look-back: the window must be at least this, and a
/// power of two (inflaters wrap around it)
pub const WINDOW: usize = 32 * 1024;

/// What one step did
pub struct Step {
    pub status: Status,
    /// Input bytes used
    pub read: usize,
    /// Bytes written into the window, at the position it was given
    pub written: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// The end of the deflate data
    Done,
    /// Give it more input
    NeedsInput,
    /// It filled the window up to its end: call again (at position 0)
    HasMoreOutput,
    Failed,
}

/// Inflate some of `input` into `window` from `position` (to at most the
/// window's end); the window keeps what was written before, which later
/// output refers back to. `more_input`: false once `input` holds the rest
pub trait InflateStep {
    fn step(&mut self, input: &[u8], window: &mut [u8], position: usize, more_input: bool) -> Step;
}

/// The inflated bytes of a .gz file, read from `inner`
pub struct GzipReader<'w, R, S> {
    inner: R,
    inflate: S,
    window: &'w mut [u8],
    /// Where the next inflated bytes go in the window
    position: usize,
    /// Inflated bytes not handed out yet: window[ready..ready + ready_len]
    ready: usize,
    ready_len: usize,
    input: Vec<u8>,
    /// input[input_start..input_end] not inflated yet
    input_start: usize,
    input_end: usize,
    input_ended: bool,
    done: bool,
}

impl<'w, R: Read, S: InflateStep> GzipReader<'w, R, S> {
    /// Read the gzip header from `inner`; then this reads the inflated
    /// bytes. `window`: WINDOW bytes
    pub fn new(mut inner: R, inflate: S, window: &'w mut [u8]) -> io::Result<Self> {
        if window.len() != WINDOW {
            return Err(invalid("inflate window not 32KB"));
        }
        skip_header(&mut inner)?;
        Ok(GzipReader {
            inner,
            inflate,
            window,
            position: 0,
            ready: 0,
            ready_len: 0,
            input: vec![0u8; 4096],
            input_start: 0,
            input_end: 0,
            input_ended: false,
            done: false,
        })
    }
}

impl<R: Read, S: InflateStep> Read for GzipReader<'_, R, S> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            if self.ready_len > 0 {
                let count = self.ready_len.min(buf.len());
                buf[..count].copy_from_slice(&self.window[self.ready..self.ready + count]);
                self.ready += count;
                self.ready_len -= count;
                return Ok(count);
            }
            if self.done {
                return Ok(0);
            }
            if self.input_start == self.input_end && !self.input_ended {
                let read = self.inner.read(&mut self.input)?;
                self.input_start = 0;
                self.input_end = read;
                self.input_ended = read == 0;
            }

            let step = self.inflate.step(
                &self.input[self.input_start..self.input_end],
                self.window,
                self.position,
                !self.input_ended,
            );
            self.input_start += step.read;
            self.ready = self.position;
            self.ready_len = step.written;
            self.position = (self.position + step.written) % WINDOW;
            match step.status {
                Status::Done => self.done = true,
                Status::Failed => return Err(invalid("deflate data corrupt")),
                Status::NeedsInput if self.input_ended && step.written == 0 => {
                    return Err(io::ErrorKind::UnexpectedEof.into())
                }
                Status::NeedsInput | Status::HasMoreOutput => {}
            }
        }
    }
}

/// The header (RFC 1952): magic, deflate, flags, time, then the optional
/// fields the flags announce. `gzip` writes the file's name (FNAME)
fn skip_header(inner: &mut impl Read) -> io::Result<()> {
    const FHCRC: u8 = 2;
    const FEXTRA: u8 = 4;
    const FNAME: u8 = 8;
    const FCOMMENT: u8 = 16;

    let mut fixed = [0u8; 10];
    inner.read_exact(&mut fixed)?;
    if fixed[0..3] != [0x1f, 0x8b, 8] {
        return Err(invalid("not a gzip file"));
    }
    let flags = fixed[3];
    if flags & FEXTRA != 0 {
        let mut len = [0u8; 2];
        inner.read_exact(&mut len)?;
        io::copy(
            &mut inner.take(u16::from_le_bytes(len) as u64),
            &mut io::sink(),
        )?;
    }
    for flag in [FNAME, FCOMMENT] {
        if flags & flag != 0 {
            let mut byte = [0u8; 1];
            loop {
                inner.read_exact(&mut byte)?;
                if byte[0] == 0 {
                    break;
                }
            }
        }
    }
    if flags & FHCRC != 0 {
        inner.read_exact(&mut [0u8; 2])?;
    }
    Ok(())
}

fn invalid(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what)
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use miniz_oxide::inflate::core::{decompress, inflate_flags, DecompressorOxide};
    use miniz_oxide::inflate::TINFLStatus;

    /// The same inflate as the ROM's, in Rust
    pub struct Oxide(pub Box<DecompressorOxide>);

    impl InflateStep for Oxide {
        fn step(
            &mut self,
            input: &[u8],
            window: &mut [u8],
            position: usize,
            more_input: bool,
        ) -> Step {
            let flags = if more_input {
                inflate_flags::TINFL_FLAG_HAS_MORE_INPUT
            } else {
                0
            };
            let (status, read, written) = decompress(&mut self.0, input, window, position, flags);
            let status = match status {
                TINFLStatus::Done => Status::Done,
                TINFLStatus::NeedsMoreInput => Status::NeedsInput,
                TINFLStatus::HasMoreOutput => Status::HasMoreOutput,
                _ => Status::Failed,
            };
            Step {
                status,
                read,
                written,
            }
        }
    }

    pub fn oxide() -> Oxide {
        Oxide(Box::default())
    }

    /// `bytes` gzipped as the gzip tool would, file name and all
    pub fn gzip(bytes: &[u8]) -> Vec<u8> {
        let mut gz = vec![0x1f, 0x8b, 8, 8, 0, 0, 0, 0, 2, 3];
        gz.extend_from_slice(b"bundle.tar\0");
        gz.extend(miniz_oxide::deflate::compress_to_vec(bytes, 9));
        gz.extend_from_slice(&[0u8; 8]); // CRC-32 and length: not checked
        gz
    }

    /// Bytes that compress, but not to nothing, and are longer than the
    /// window several times over
    fn sample() -> Vec<u8> {
        (0..200_000u32)
            .map(|i| ((i * 7) ^ (i >> 5)) as u8)
            .collect()
    }

    #[test]
    fn inflates_what_gzip_wrote() {
        let original = sample();
        let gz = gzip(&original);
        let mut window = vec![0u8; WINDOW];
        let mut reader = GzipReader::new(&gz[..], oxide(), &mut window).unwrap();
        let mut out = Vec::new();
        reader.read_to_end(&mut out).unwrap();
        assert_eq!(out, original);
    }

    #[test]
    fn small_reads_get_the_same_bytes() {
        let original = sample();
        let gz = gzip(&original);
        let mut window = vec![0u8; WINDOW];
        let mut reader = GzipReader::new(&gz[..], oxide(), &mut window).unwrap();
        let mut out = Vec::new();
        let mut buf = [0u8; 333];
        loop {
            let read = reader.read(&mut buf).unwrap();
            if read == 0 {
                break;
            }
            out.extend_from_slice(&buf[..read]);
        }
        assert_eq!(out, original);
    }

    #[test]
    fn not_gzip_is_an_error() {
        let mut window = vec![0u8; WINDOW];
        assert!(GzipReader::new(&b"plain text, no magic"[..], oxide(), &mut window).is_err());
    }

    #[test]
    fn cut_short_is_an_error() {
        let gz = gzip(&sample());
        let mut window = vec![0u8; WINDOW];
        let mut reader = GzipReader::new(&gz[..gz.len() / 2], oxide(), &mut window).unwrap();
        let mut out = Vec::new();
        assert!(reader.read_to_end(&mut out).is_err());
    }
}
