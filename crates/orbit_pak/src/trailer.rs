//! The trailer of a pak glued to the end of an executable.
//!
//! `[executable][pak][pak length: u64 LE]["ORBITEND"]`: a player finds its
//! own game by reading the last [`TRAILER_LEN`] bytes of its executable.

/// Size of the trailer.
pub const TRAILER_LEN: usize = 16;
const MAGIC: &[u8; 8] = b"ORBITEND";

/// The trailer bytes for a pak of `pak_len` bytes.
pub fn encode(pak_len: u64) -> [u8; TRAILER_LEN] {
    let mut out = [0u8; TRAILER_LEN];
    out[..8].copy_from_slice(&pak_len.to_le_bytes());
    out[8..].copy_from_slice(MAGIC);
    out
}

/// Length of the pak announced by `trailer`, if it is one.
pub fn decode(trailer: &[u8]) -> Option<u64> {
    if trailer.len() != TRAILER_LEN || &trailer[8..] != MAGIC {
        return None;
    }
    Some(u64::from_le_bytes(trailer[..8].try_into().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_garbage() {
        assert_eq!(decode(&encode(123_456_789_012)), Some(123_456_789_012));
        assert_eq!(decode(&[0u8; TRAILER_LEN]), None);
        assert_eq!(decode(b"short"), None);
    }
}
