//! The pak container.
//!
//! ```text
//! offset 0   "ORBITPAK"            magic (8 bytes)
//!        8   format version        u32 LE  (currently 1)
//!       12   index length          u32 LE
//!       16   data length           u64 LE
//!       24   index                 postcard: metadata + entry list
//!  24+index  data                  the entries, back to back
//! ```
//!
//! Each [`Entry`] gives the offset (relative to the start of the data), the
//! stored and the original length, the [`Codec`] and the CRC-32 of the
//! original bytes. A reader needs `HEADER_LEN + index length` bytes to know
//! everything, then reads entries independently: that is what lets the web
//! player fetch the index alone and the assets one by one with HTTP range
//! requests.
//!
//! A newer format version is refused with a clear error (never guessed at).

use std::collections::HashMap;
use std::io;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::crc::crc32;

/// First bytes of every pak.
pub const MAGIC: &[u8; 8] = b"ORBITPAK";
/// Current container version.
pub const FORMAT_VERSION: u32 = 1;
/// Size of the fixed header.
pub const HEADER_LEN: usize = 24;
/// Entries smaller than this are never compressed (not worth the CPU).
const MIN_COMPRESS_LEN: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PakError {
    #[error("not an OrbitEngine pak (bad magic)")]
    BadMagic,
    #[error("pak format version {found} is not supported (this build reads up to {FORMAT_VERSION})")]
    UnsupportedVersion { found: u32 },
    #[error("the pak is truncated")]
    Truncated,
    #[error("the pak index is corrupt: {0}")]
    Index(String),
    #[error("`{0}` is not in the pak")]
    Missing(String),
    #[error("`{path}` is corrupt: {reason}")]
    Corrupt { path: String, reason: String },
    #[error("invalid entry path `{0}`")]
    InvalidPath(String),
    #[error("duplicate entry `{0}`")]
    Duplicate(String),
}

/// The fixed-size header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub version: u32,
    pub index_len: u32,
    pub data_len: u64,
}

impl Header {
    /// Parses the first [`HEADER_LEN`] bytes of a pak.
    pub fn parse(bytes: &[u8]) -> Result<Self, PakError> {
        if bytes.len() < MAGIC.len() {
            return Err(PakError::Truncated);
        }
        if &bytes[..MAGIC.len()] != MAGIC {
            return Err(PakError::BadMagic);
        }
        if bytes.len() < HEADER_LEN {
            return Err(PakError::Truncated);
        }
        let version = u32::from_le_bytes(bytes[8..12].try_into().expect("4 bytes"));
        if version == 0 || version > FORMAT_VERSION {
            return Err(PakError::UnsupportedVersion { found: version });
        }
        Ok(Self {
            version,
            index_len: u32::from_le_bytes(bytes[12..16].try_into().expect("4 bytes")),
            data_len: u64::from_le_bytes(bytes[16..24].try_into().expect("8 bytes")),
        })
    }

    pub fn encode(&self) -> [u8; HEADER_LEN] {
        let mut out = [0u8; HEADER_LEN];
        out[..8].copy_from_slice(MAGIC);
        out[8..12].copy_from_slice(&self.version.to_le_bytes());
        out[12..16].copy_from_slice(&self.index_len.to_le_bytes());
        out[16..24].copy_from_slice(&self.data_len.to_le_bytes());
        out
    }

    /// Where the index lives in the pak: `HEADER_LEN..index_end()`.
    pub fn index_end(&self) -> u64 {
        HEADER_LEN as u64 + u64::from(self.index_len)
    }

    /// Where the data starts (entry offsets are relative to it).
    pub fn data_start(&self) -> u64 {
        self.index_end()
    }

    /// Total size of the pak.
    pub fn total_len(&self) -> u64 {
        self.data_start() + self.data_len
    }
}

/// How an entry is stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Codec {
    Raw,
    /// LZ4 block format, original length in the entry.
    Lz4,
}

/// Whether the writer compresses entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Compression {
    None,
    /// LZ4 for the entries that gain at least 10 % (images and sounds are
    /// already compressed and stay raw).
    #[default]
    Lz4,
}

/// One file of the pak.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// `/`-separated path.
    pub path: String,
    /// Offset of the stored bytes, relative to the start of the data.
    pub offset: u64,
    pub stored_len: u64,
    pub raw_len: u64,
    pub codec: Codec,
    /// CRC-32 of the original bytes.
    pub crc: u32,
}

impl Entry {
    /// Original bytes of the entry, from the `stored` bytes read at
    /// [`PakIndex::stored_range`]. Verifies the length and the CRC.
    pub fn decode(&self, stored: &[u8]) -> Result<Vec<u8>, PakError> {
        let corrupt = |reason: &str| PakError::Corrupt {
            path: self.path.clone(),
            reason: reason.to_owned(),
        };
        if stored.len() as u64 != self.stored_len {
            return Err(corrupt("wrong stored length"));
        }
        let raw = match self.codec {
            Codec::Raw => stored.to_vec(),
            Codec::Lz4 => lz4_flex::block::decompress(
                stored,
                usize::try_from(self.raw_len).map_err(|_| corrupt("too large"))?,
            )
            .map_err(|e| corrupt(&format!("lz4: {e}")))?,
        };
        if raw.len() as u64 != self.raw_len {
            return Err(corrupt("wrong length"));
        }
        if crc32(&raw) != self.crc {
            return Err(corrupt("checksum mismatch"));
        }
        Ok(raw)
    }
}

#[derive(Serialize, Deserialize)]
struct IndexData {
    meta: Vec<(String, String)>,
    entries: Vec<Entry>,
}

/// The parsed index: metadata and entries, with lookup by path.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PakIndex {
    meta: Vec<(String, String)>,
    entries: Vec<Entry>,
    lookup: HashMap<String, usize>,
}

impl PakIndex {
    fn from_data(data: IndexData) -> Result<Self, PakError> {
        let mut lookup = HashMap::with_capacity(data.entries.len());
        for (i, entry) in data.entries.iter().enumerate() {
            if lookup.insert(entry.path.clone(), i).is_some() {
                return Err(PakError::Duplicate(entry.path.clone()));
            }
        }
        Ok(Self {
            meta: data.meta,
            entries: data.entries,
            lookup,
        })
    }

    /// Parses the index bytes (`HEADER_LEN..index_end` of the pak).
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, PakError> {
        let data: IndexData =
            postcard::from_bytes(bytes).map_err(|e| PakError::Index(e.to_string()))?;
        Self::from_data(data)
    }

    pub fn entry(&self, path: &str) -> Option<&Entry> {
        self.lookup.get(path).map(|&i| &self.entries[i])
    }

    pub fn contains(&self, path: &str) -> bool {
        self.lookup.contains_key(path)
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// A metadata value (`name`, `id`, `version`...), see [`PakWriter::meta`].
    pub fn meta(&self, key: &str) -> Option<&str> {
        self.meta
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    pub fn all_meta(&self) -> &[(String, String)] {
        &self.meta
    }

    /// Absolute position (from the start of the pak) and length of the
    /// stored bytes of `entry`.
    pub fn stored_range(&self, header: &Header, entry: &Entry) -> (u64, u64) {
        (header.data_start() + entry.offset, entry.stored_len)
    }

    /// Reads and decodes an entry from a complete pak held in memory.
    pub fn read_from(&self, pak: &[u8], path: &str) -> Result<Vec<u8>, PakError> {
        let header = Header::parse(pak)?;
        let entry = self
            .entry(path)
            .ok_or_else(|| PakError::Missing(path.to_owned()))?;
        let (start, len) = self.stored_range(&header, entry);
        let end = start.checked_add(len).ok_or(PakError::Truncated)?;
        let stored = pak
            .get(start as usize..end as usize)
            .ok_or(PakError::Truncated)?;
        entry.decode(stored)
    }
}

/// Parses the header and the index of a pak held in memory (or a prefix of
/// it that contains the whole index).
pub fn parse_prefix(bytes: &[u8]) -> Result<(Header, PakIndex), PakError> {
    let header = Header::parse(bytes)?;
    let end = header.index_end() as usize;
    let index_bytes = bytes.get(HEADER_LEN..end).ok_or(PakError::Truncated)?;
    Ok((header, PakIndex::from_bytes(index_bytes)?))
}

/// What a finished pak contains.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WriteStats {
    pub entries: usize,
    /// Sum of the original sizes.
    pub raw_bytes: u64,
    /// Sum of the stored sizes.
    pub stored_bytes: u64,
    /// Size of the whole pak (header and index included).
    pub pak_bytes: u64,
}

/// Builds a pak in memory, then writes it.
pub struct PakWriter {
    compression: Compression,
    meta: Vec<(String, String)>,
    entries: Vec<Entry>,
    blobs: Vec<Vec<u8>>,
    next_offset: u64,
}

/// Extensions of formats that are already compressed: stored raw.
fn is_precompressed(path: &str) -> bool {
    let ext = path
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "webp" | "ogg" | "oga" | "mp3" | "flac" | "opus" | "zip" | "gz"
    )
}

fn valid_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && !path.contains(':')
        && path.split('/').all(|part| !part.is_empty() && part != "..")
}

impl PakWriter {
    pub fn new(compression: Compression) -> Self {
        Self {
            compression,
            meta: Vec::new(),
            entries: Vec::new(),
            blobs: Vec::new(),
            next_offset: 0,
        }
    }

    /// Sets a metadata key (replacing a previous value).
    pub fn meta(&mut self, key: &str, value: impl Into<String>) {
        let value = value.into();
        match self.meta.iter_mut().find(|(k, _)| k == key) {
            Some(slot) => slot.1 = value,
            None => self.meta.push((key.to_owned(), value)),
        }
    }

    /// Adds a file. Paths are relative, `/`-separated, without `..`.
    pub fn add(&mut self, path: &str, data: &[u8]) -> Result<(), PakError> {
        if !valid_path(path) {
            return Err(PakError::InvalidPath(path.to_owned()));
        }
        if self.entries.iter().any(|e| e.path == path) {
            return Err(PakError::Duplicate(path.to_owned()));
        }
        let (codec, stored) = self.encode(path, data);
        let entry = Entry {
            path: path.to_owned(),
            offset: self.next_offset,
            stored_len: stored.len() as u64,
            raw_len: data.len() as u64,
            codec,
            crc: crc32(data),
        };
        self.next_offset += entry.stored_len;
        self.entries.push(entry);
        self.blobs.push(stored);
        Ok(())
    }

    fn encode(&self, path: &str, data: &[u8]) -> (Codec, Vec<u8>) {
        if self.compression == Compression::Lz4
            && data.len() >= MIN_COMPRESS_LEN
            && !is_precompressed(path)
        {
            let packed = lz4_flex::block::compress(data);
            if packed.len() * 10 <= data.len() * 9 {
                return (Codec::Lz4, packed);
            }
        }
        (Codec::Raw, data.to_vec())
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Writes the pak.
    pub fn write_to(self, out: &mut impl io::Write) -> io::Result<WriteStats> {
        let stats_entries = self.entries.len();
        let raw_bytes = self.entries.iter().map(|e| e.raw_len).sum();
        let stored_bytes = self.next_offset;
        let index = postcard::to_stdvec(&IndexData {
            meta: self.meta,
            entries: self.entries,
        })
        .map_err(|e| io::Error::other(e.to_string()))?;
        let index_len = u32::try_from(index.len())
            .map_err(|_| io::Error::other("the pak index is larger than 4 GiB"))?;
        let header = Header {
            version: FORMAT_VERSION,
            index_len,
            data_len: stored_bytes,
        };
        out.write_all(&header.encode())?;
        out.write_all(&index)?;
        for blob in &self.blobs {
            out.write_all(blob)?;
        }
        Ok(WriteStats {
            entries: stats_entries,
            raw_bytes,
            stored_bytes,
            pak_bytes: header.total_len(),
        })
    }

    /// The pak as bytes.
    pub fn to_bytes(self) -> (Vec<u8>, WriteStats) {
        let mut out = Vec::new();
        let stats = self.write_to(&mut out).expect("writing to a Vec cannot fail");
        (out, stats)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> (Vec<u8>, WriteStats) {
        let mut w = PakWriter::new(Compression::Lz4);
        w.meta("name", "Demo");
        w.add("a.txt", &b"hello ".repeat(100)).unwrap();
        w.add("img/b.png", &[7u8; 300]).unwrap();
        w.add("empty.bin", b"").unwrap();
        w.to_bytes()
    }

    #[test]
    fn round_trip_with_compression() {
        let (bytes, stats) = sample();
        assert_eq!(stats.entries, 3);
        assert_eq!(stats.pak_bytes, bytes.len() as u64);
        assert!(stats.stored_bytes < stats.raw_bytes);
        let (header, index) = parse_prefix(&bytes).unwrap();
        assert_eq!(header.total_len(), bytes.len() as u64);
        assert_eq!(index.meta("name"), Some("Demo"));
        assert_eq!(index.entry("a.txt").unwrap().codec, Codec::Lz4);
        // Already compressed formats stay raw.
        assert_eq!(index.entry("img/b.png").unwrap().codec, Codec::Raw);
        assert_eq!(index.read_from(&bytes, "img/b.png").unwrap(), vec![7u8; 300]);
        assert_eq!(index.read_from(&bytes, "empty.bin").unwrap(), b"");
        assert_eq!(index.read_from(&bytes, "a.txt").unwrap(), b"hello ".repeat(100));
    }

    #[test]
    fn no_compression_keeps_everything_raw() {
        let mut w = PakWriter::new(Compression::None);
        w.add("a.txt", &[1u8; 500]).unwrap();
        let (bytes, stats) = w.to_bytes();
        assert_eq!(stats.stored_bytes, stats.raw_bytes);
        let (_, index) = parse_prefix(&bytes).unwrap();
        assert_eq!(index.entry("a.txt").unwrap().codec, Codec::Raw);
    }

    #[test]
    fn a_prefix_with_the_whole_index_is_enough() {
        let (bytes, _) = sample();
        let header = Header::parse(&bytes).unwrap();
        let prefix = &bytes[..header.index_end() as usize];
        let (_, index) = parse_prefix(prefix).unwrap();
        // The entry can be decoded from just its stored range.
        let e = index.entry("a.txt").unwrap();
        let (start, len) = index.stored_range(&header, e);
        let stored = &bytes[start as usize..(start + len) as usize];
        assert!(e.decode(stored).is_ok());
        assert_eq!(parse_prefix(&bytes[..10]).unwrap_err(), PakError::Truncated);
    }

    #[test]
    fn bad_input_is_refused() {
        assert_eq!(Header::parse(b"nope").unwrap_err(), PakError::Truncated);
        assert_eq!(
            Header::parse(b"NOTAPAKFILE-----------------").unwrap_err(),
            PakError::BadMagic
        );
        let (mut bytes, _) = sample();
        bytes[8] = 9; // a future version
        assert_eq!(
            Header::parse(&bytes).unwrap_err(),
            PakError::UnsupportedVersion { found: 9 }
        );
        let mut w = PakWriter::new(Compression::None);
        assert!(w.add("../x", b"").is_err());
        assert!(w.add("/x", b"").is_err());
        assert!(w.add("a\\b", b"").is_err());
        w.add("x", b"1").unwrap();
        assert_eq!(w.add("x", b"2").unwrap_err(), PakError::Duplicate("x".into()));
    }

    #[test]
    fn corruption_is_detected() {
        let (mut bytes, _) = sample();
        let (_, index) = parse_prefix(&bytes).unwrap();
        let header = Header::parse(&bytes).unwrap();
        let e = index.entry("img/b.png").unwrap().clone();
        let (start, _) = index.stored_range(&header, &e);
        bytes[start as usize] ^= 0xFF;
        assert!(matches!(
            index.read_from(&bytes, "img/b.png"),
            Err(PakError::Corrupt { .. })
        ));
        assert!(matches!(
            index.read_from(&bytes, "missing"),
            Err(PakError::Missing(_))
        ));
        bytes.truncate(bytes.len() - 5);
        assert!(index.read_from(&bytes, "img/b.png").is_err());
    }
}
