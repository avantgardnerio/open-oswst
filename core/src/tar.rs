//! Reading a tar archive as a stream, the way an update bundle is read
//! (bundle.rs): front to back once, never holding a whole file. A tar is a
//! 512-byte header, then the file's bytes padded to a multiple of 512, then
//! the next header, and two blocks of zeros at the end. Only ustar, what
//! `tar --format=ustar` writes (scripts/publish-firmware.sh).

use std::io::{self, Read};

const BLOCK: u64 = 512;

/// One file (or folder) in the archive
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// As stored, less any "./" at the front: "www/app.js"
    pub path: String,
    pub size: u64,
    /// A plain file. Anything else (a folder, a link) has no bytes to keep
    pub is_file: bool,
}

pub struct TarReader<R> {
    inner: R,
    /// Bytes of the current entry not read yet
    left: u64,
    /// Zeros after the current entry, up to the next header
    padding: u64,
}

impl<R: Read> TarReader<R> {
    pub fn new(inner: R) -> TarReader<R> {
        TarReader {
            inner,
            left: 0,
            padding: 0,
        }
    }

    /// The next entry's header, after skipping whatever of the current one
    /// wasn't read. None at the end of the archive. Its bytes are then read
    /// from this reader (Read), up to `size`
    pub fn next_entry(&mut self) -> io::Result<Option<Entry>> {
        skip(&mut self.inner, self.left + self.padding)?;
        self.left = 0;
        self.padding = 0;

        let mut header = [0u8; BLOCK as usize];
        self.inner.read_exact(&mut header)?;
        // The end: a block of zeros (a second one follows; nothing to read)
        if header.iter().all(|&byte| byte == 0) {
            return Ok(None);
        }
        let entry = parse_header(&header)?;
        self.left = entry.size;
        self.padding = (BLOCK - entry.size % BLOCK) % BLOCK;
        Ok(Some(entry))
    }
}

/// The current entry's bytes; 0 at its end
impl<R: Read> Read for TarReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let most = buf.len().min(self.left.min(usize::MAX as u64) as usize);
        if most == 0 {
            return Ok(0);
        }
        let read = self.inner.read(&mut buf[..most])?;
        if read == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        self.left -= read as u64;
        Ok(read)
    }
}

/// A ustar header: the name (split into prefix and name if long), the size
/// in octal, the type, and a checksum that catches a reader gone out of step
fn parse_header(header: &[u8; BLOCK as usize]) -> io::Result<Entry> {
    if &header[257..262] != b"ustar" {
        return Err(invalid("not a ustar header"));
    }
    // The checksum: every byte summed, with its own 8 counted as spaces
    let stored = octal(&header[148..156])?;
    let sum: u64 = header
        .iter()
        .enumerate()
        .map(|(at, &byte)| if (148..156).contains(&at) { b' ' } else { byte } as u64)
        .sum();
    if stored != sum {
        return Err(invalid("tar header checksum"));
    }

    let name = text(&header[0..100])?;
    let prefix = text(&header[345..500])?;
    let path = if prefix.is_empty() {
        name
    } else {
        format!("{}/{}", prefix, name)
    };
    let path = path.strip_prefix("./").unwrap_or(&path).to_string();
    // '0' a file, and NUL too (before ustar)
    let is_file = matches!(header[156], b'0' | 0);
    Ok(Entry {
        path,
        size: octal(&header[124..136])?,
        is_file,
    })
}

/// A field of text, up to its first NUL
fn text(field: &[u8]) -> io::Result<String> {
    let end = field
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(field.len());
    String::from_utf8(field[..end].to_vec()).map_err(|_| invalid("tar name not UTF-8"))
}

/// A number field: octal digits, padded with spaces or NULs
fn octal(field: &[u8]) -> io::Result<u64> {
    let digits: &[u8] = field
        .split(|&byte| byte == 0 || byte == b' ')
        .find(|part| !part.is_empty())
        .unwrap_or(&[]);
    let digits = std::str::from_utf8(digits).map_err(|_| invalid("tar number"))?;
    if digits.is_empty() {
        return Ok(0);
    }
    u64::from_str_radix(digits, 8).map_err(|_| invalid("tar number"))
}

/// Read and drop `count` bytes
fn skip(reader: &mut impl Read, count: u64) -> io::Result<()> {
    let copied = io::copy(&mut reader.take(count), &mut io::sink())?;
    if copied < count {
        return Err(io::ErrorKind::UnexpectedEof.into());
    }
    Ok(())
}

fn invalid(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One ustar header, as `tar` writes it
    fn header(path: &str, size: usize, kind: u8) -> Vec<u8> {
        let mut header = vec![0u8; BLOCK as usize];
        header[..path.len()].copy_from_slice(path.as_bytes());
        header[100..108].copy_from_slice(b"0000644\0");
        header[124..136].copy_from_slice(format!("{:011o}\0", size).as_bytes());
        header[156] = kind;
        header[257..263].copy_from_slice(b"ustar\0");
        header[263..265].copy_from_slice(b"00");
        header[148..156].copy_from_slice(b"        ");
        let sum: u32 = header.iter().map(|&byte| byte as u32).sum();
        header[148..156].copy_from_slice(format!("{:06o}\0 ", sum).as_bytes());
        header
    }

    /// An archive of (path, bytes) files, after a folder entry
    fn archive(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut tar = header("./www/", 0, b'5');
        for (path, bytes) in files {
            tar.extend(header(path, bytes.len(), b'0'));
            tar.extend_from_slice(bytes);
            let padding = (BLOCK as usize - bytes.len() % BLOCK as usize) % BLOCK as usize;
            tar.extend(vec![0u8; padding]);
        }
        tar.extend(vec![0u8; 2 * BLOCK as usize]);
        tar
    }

    #[test]
    fn entries_and_their_bytes_in_order() {
        let big = vec![7u8; 1300]; // more than two blocks: padding to skip
        let tar = archive(&[("./firmware.bin", &big), ("./www/app.js", b"hello")]);
        let mut reader = TarReader::new(&tar[..]);

        let folder = reader.next_entry().unwrap().unwrap();
        assert_eq!(folder.path, "www/");
        assert!(!folder.is_file);

        let firmware = reader.next_entry().unwrap().unwrap();
        assert_eq!(firmware.path, "firmware.bin");
        assert_eq!(firmware.size, 1300);
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, big);

        // Not read at all: skipped on the way to the next one
        let app = reader.next_entry().unwrap().unwrap();
        assert_eq!(app.path, "www/app.js");
        assert!(app.is_file);
        assert_eq!(reader.next_entry().unwrap(), None);
    }

    #[test]
    fn a_corrupt_header_is_an_error_not_a_guess() {
        let mut tar = archive(&[("a.txt", b"x")]);
        tar[BLOCK as usize + 3] ^= 0xff; // the file's name, after the folder
        let mut reader = TarReader::new(&tar[..]);
        reader.next_entry().unwrap();
        assert!(reader.next_entry().is_err());
    }

    #[test]
    fn a_short_archive_is_an_error() {
        let tar = archive(&[("a.txt", &[1u8; 600])]);
        let mut reader = TarReader::new(&tar[..BLOCK as usize * 3]); // folder, header, part
        reader.next_entry().unwrap();
        reader.next_entry().unwrap();
        let mut bytes = Vec::new();
        assert!(reader.read_to_end(&mut bytes).is_err());
    }
}
