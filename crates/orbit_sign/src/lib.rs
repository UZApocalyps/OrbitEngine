//! Hashes and signatures for the plugin store, without a dependency:
//!
//! * [`sha256`] (FIPS 180-4): checksums of the downloaded plugin packages,
//!   the editor build id;
//! * [`sha512`]: what Ed25519 hashes with;
//! * [`ed25519`] (RFC 8032): key pair from a 32-byte seed, signing (the
//!   release tooling), verification (the editor). A port of TweetNaCl's
//!   `crypto_sign` (public domain, small and audited), plus the `S < L`
//!   check of RFC 8032 (no malleable signatures).
//!
//! Verification only handles public data (catalog signatures), so the
//! constant-time care of the signing side matters less; signing runs once per
//! build, on the release machine.

pub mod ed25519;
pub mod sha256;
pub mod sha512;

/// Lowercase hex.
pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(DIGITS[(b >> 4) as usize] as char);
        out.push(DIGITS[(b & 15) as usize] as char);
    }
    out
}

/// Parses hex (either case, surrounding spaces ignored) into exactly `N` bytes.
pub fn from_hex<const N: usize>(text: &str) -> Option<[u8; N]> {
    let text = text.trim().as_bytes();
    if text.len() != N * 2 {
        return None;
    }
    let digit = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    };
    let mut out = [0u8; N];
    for (i, pair) in text.chunks(2).enumerate() {
        out[i] = (digit(pair[0])? << 4) | digit(pair[1])?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips() {
        assert_eq!(hex(&[0, 0xab, 0xff]), "00abff");
        assert_eq!(from_hex::<3>("00ABff"), Some([0, 0xab, 0xff]));
        assert_eq!(from_hex::<3>(" 00abff\n"), Some([0, 0xab, 0xff]));
        assert_eq!(from_hex::<3>("00abf"), None);
        assert_eq!(from_hex::<2>("zz00"), None);
    }
}
