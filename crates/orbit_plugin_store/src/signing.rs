//! Signed builds: what is signed, the official key, the verifier.
//!
//! A catalog build is signed with Ed25519 over [`build_message`] (plugin id,
//! version, editor build id, SHA-256 of the package). The signature is
//! checked against the catalog's SHA-256 before downloading, and the
//! downloaded file's SHA-256 must equal it: a package can only be installed
//! if its exact bytes were signed for this plugin, version and editor build.

use crate::catalog::Build;
use crate::sources::Trust;

/// The official public key (Ed25519, hex), generated with
/// `cargo xtask plugin-key gen`; its private key is the GitHub secret
/// `ORBIT_PLUGIN_SIGNING_KEY` of `UZApocalyps/OrbitEnginePlugins` and is
/// never committed.
pub const OFFICIAL_PUBLIC_KEY: &str =
    "981061d013fdb4e88bf2a2a8ff2df54564cf159a7a926dafb4db102939022b43";

/// The message a build signature covers (every line ends with `\n`).
pub fn build_message(id: &str, version: &str, fingerprint: &str, sha256: &str) -> String {
    format!(
        "orbit-plugin-build/v1\nid={id}\nversion={version}\nfingerprint={}\nsha256={}\n",
        fingerprint.to_ascii_lowercase(),
        sha256.to_ascii_lowercase()
    )
}

/// Checks signatures. The default is [`Ed25519`]; tests (or another
/// implementation) can plug their own.
pub trait SignatureVerifier: Send + Sync {
    fn verify(&self, public_key: &[u8; 32], message: &[u8], signature: &[u8; 64]) -> bool;
}

/// Ed25519 from `orbit_sign`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Ed25519;

impl SignatureVerifier for Ed25519 {
    fn verify(&self, public_key: &[u8; 32], message: &[u8], signature: &[u8; 64]) -> bool {
        orbit_sign::ed25519::verify(public_key, message, signature)
    }
}

/// A public key in hex.
pub fn parse_public_key(text: &str) -> Option<[u8; 32]> {
    orbit_sign::from_hex::<32>(text.trim())
}

/// The official key.
pub fn official_key() -> Option<[u8; 32]> {
    parse_public_key(OFFICIAL_PUBLIC_KEY).filter(|k| k.iter().any(|&b| b != 0))
}

/// The private key (32-byte seed) of a key file: the first line that is 64
/// hex characters (`#` comments and other lines ignored), so the file, or
/// just its key line, can be the GitHub secret.
pub fn parse_private_key(text: &str) -> Option<[u8; 32]> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.starts_with('#'))
        .find_map(orbit_sign::from_hex::<32>)
}

/// The key file `cargo xtask plugin-key gen` writes.
pub fn private_key_file(seed: &[u8; 32]) -> String {
    format!(
        "# OrbitEngine plugin signing key (Ed25519 seed). SECRET: never commit it.\n# public key: {}\n{}\n",
        orbit_sign::hex(&orbit_sign::ed25519::public_key(seed)),
        orbit_sign::hex(seed)
    )
}

/// The public key (hex) of a seed.
pub fn public_key_hex(seed: &[u8; 32]) -> String {
    orbit_sign::hex(&orbit_sign::ed25519::public_key(seed))
}

/// Signs a build (hex signature).
pub fn sign_build(
    seed: &[u8; 32],
    id: &str,
    version: &str,
    fingerprint: &str,
    sha256: &str,
) -> String {
    let message = build_message(id, version, fingerprint, sha256);
    orbit_sign::hex(&orbit_sign::ed25519::sign(seed, message.as_bytes()))
}

/// What the signature check found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignatureState {
    /// Valid, by the source's key (official or trusted).
    Valid,
    /// The source has no key: nothing checked (warned in the interface).
    NotChecked,
}

/// Why a build is refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SignatureError {
    /// The source requires signatures and the build has none.
    Missing,
    /// The signature does not match the source's key.
    Invalid,
    /// The source should have a key and has none usable (e.g. an editor built
    /// without the official key).
    NoKey,
}

impl std::fmt::Display for SignatureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing => write!(f, "the build is not signed"),
            Self::Invalid => write!(f, "the signature does not match the source's key"),
            Self::NoKey => write!(f, "no public key to check the signature"),
        }
    }
}

/// Checks the signature of `build` (version `version` of `id`) for a source
/// of `trust` with `key`.
pub fn check_build(
    trust: Trust,
    key: Option<&[u8; 32]>,
    id: &str,
    version: &str,
    build: &Build,
    verifier: &dyn SignatureVerifier,
) -> Result<SignatureState, SignatureError> {
    if trust == Trust::Unsigned {
        return Ok(SignatureState::NotChecked);
    }
    let key = key.ok_or(SignatureError::NoKey)?;
    let signature = build.signature.as_deref().ok_or(SignatureError::Missing)?;
    let signature = orbit_sign::from_hex::<64>(signature).ok_or(SignatureError::Invalid)?;
    let message = build_message(id, version, &build.fingerprint, &build.sha256);
    if verifier.verify(key, message.as_bytes(), &signature) {
        Ok(SignatureState::Valid)
    } else {
        Err(SignatureError::Invalid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn sample_build(seed: Option<&[u8; 32]>) -> Build {
        let fingerprint = "0123456789abcdef0123456789abcdef".to_owned();
        let sha256 = "ab".repeat(32);
        Build {
            editor_version: "0.4.0".into(),
            signature: seed.map(|s| sign_build(s, "acme.hello", "1.0.0", &fingerprint, &sha256)),
            fingerprint,
            target: "x86_64-pc-windows-msvc".into(),
            profile: "release".into(),
            url: "https://example.org/a.orbitplugin".into(),
            sha256,
            size: 10,
        }
    }

    struct Never;
    impl SignatureVerifier for Never {
        fn verify(&self, _: &[u8; 32], _: &[u8], _: &[u8; 64]) -> bool {
            false
        }
    }

    #[test]
    fn signatures_are_checked_by_trust_level() {
        let seed = [9u8; 32];
        let key = parse_public_key(&public_key_hex(&seed)).unwrap();
        let signed = sample_build(Some(&seed));
        let unsigned = sample_build(None);
        let v = Ed25519;
        for trust in [Trust::Official, Trust::Trusted] {
            assert_eq!(
                check_build(trust, Some(&key), "acme.hello", "1.0.0", &signed, &v),
                Ok(SignatureState::Valid)
            );
            // Another version, id or hash: invalid.
            assert_eq!(
                check_build(trust, Some(&key), "acme.hello", "1.0.1", &signed, &v),
                Err(SignatureError::Invalid)
            );
            assert_eq!(
                check_build(trust, Some(&key), "acme.evil", "1.0.0", &signed, &v),
                Err(SignatureError::Invalid)
            );
            let mut other_hash = signed.clone();
            other_hash.sha256 = "cd".repeat(32);
            assert_eq!(
                check_build(trust, Some(&key), "acme.hello", "1.0.0", &other_hash, &v),
                Err(SignatureError::Invalid)
            );
            let mut other_editor = signed.clone();
            other_editor.fingerprint = "f".repeat(32);
            assert_eq!(
                check_build(trust, Some(&key), "acme.hello", "1.0.0", &other_editor, &v),
                Err(SignatureError::Invalid)
            );
            // Another key.
            let other_key = parse_public_key(&public_key_hex(&[1u8; 32])).unwrap();
            assert_eq!(
                check_build(trust, Some(&other_key), "acme.hello", "1.0.0", &signed, &v),
                Err(SignatureError::Invalid)
            );
            assert_eq!(
                check_build(trust, Some(&key), "acme.hello", "1.0.0", &unsigned, &v),
                Err(SignatureError::Missing)
            );
            assert_eq!(
                check_build(trust, None, "acme.hello", "1.0.0", &signed, &v),
                Err(SignatureError::NoKey)
            );
        }
        assert_eq!(
            check_build(Trust::Unsigned, None, "acme.hello", "1.0.0", &unsigned, &v),
            Ok(SignatureState::NotChecked)
        );
        // The verifier is pluggable.
        assert_eq!(
            check_build(
                Trust::Trusted,
                Some(&key),
                "acme.hello",
                "1.0.0",
                &signed,
                &Never
            ),
            Err(SignatureError::Invalid)
        );
    }

    #[test]
    fn key_files_round_trip() {
        let seed = [3u8; 32];
        let file = private_key_file(&seed);
        assert!(file.contains(&public_key_hex(&seed)));
        assert_eq!(parse_private_key(&file), Some(seed));
        assert_eq!(parse_private_key(&orbit_sign::hex(&seed)), Some(seed));
        assert_eq!(parse_private_key("# nothing\n"), None);
        assert_eq!(
            build_message("a.b", "1.0.0", "ABC", "DEF"),
            "orbit-plugin-build/v1\nid=a.b\nversion=1.0.0\nfingerprint=abc\nsha256=def\n"
        );
    }

    #[test]
    fn the_official_key_is_set() {
        assert!(
            official_key().is_some(),
            "OFFICIAL_PUBLIC_KEY is the placeholder"
        );
    }
}
