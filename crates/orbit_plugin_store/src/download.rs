//! Downloading a catalog build: signature checked first (it covers the
//! catalog's SHA-256), then the file streamed to
//! `<config>/plugin_store/downloads/`, its size and SHA-256 compared with the
//! catalog. Only a file that passes is returned, for the plugin installer
//! (which runs its own checks: format, descriptor, fingerprint).

use std::path::PathBuf;
use std::time::Duration;

use crate::StorePaths;
use crate::catalog::Build;
use crate::signing::{SignatureError, SignatureState, SignatureVerifier, check_build};
use crate::sources::{SourceEntry, Trust};

/// What to download: a build of `version` of `plugin_id` from `source`.
#[derive(Clone, Debug)]
pub struct DownloadPlan {
    pub source: SourceEntry,
    pub plugin_id: String,
    pub version: String,
    pub build: Build,
}

/// Why a build was not downloaded or was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DownloadError {
    Signature(SignatureError),
    Network(String),
    Http(u16),
    Size { expected: u64, got: u64 },
    Checksum { expected: String, got: String },
    Cancelled,
    Io(String),
}

impl std::fmt::Display for DownloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Signature(e) => write!(f, "{e}"),
            Self::Network(e) => write!(f, "{e}"),
            Self::Http(s) => write!(f, "HTTP {s}"),
            Self::Size { expected, got } => write!(f, "size {got} bytes instead of {expected}"),
            Self::Checksum { expected, got } => write!(f, "SHA-256 {got} instead of {expected}"),
            Self::Cancelled => write!(f, "cancelled"),
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}

/// A verified package, ready for the installer.
#[derive(Clone, Debug)]
pub struct Verified {
    pub file: PathBuf,
    pub signature: SignatureState,
    pub trust: Trust,
}

/// Downloads and verifies `plan`. `progress(received, total)` returns
/// `false` to cancel. The partial file is removed on any failure.
pub fn download(
    paths: &StorePaths,
    plan: &DownloadPlan,
    verifier: &dyn SignatureVerifier,
    progress: &dyn Fn(u64, u64) -> bool,
) -> Result<Verified, DownloadError> {
    let trust = plan.source.trust();
    let key = plan.source.key();
    let signature = check_build(
        trust,
        key.as_ref(),
        &plan.plugin_id,
        &plan.version,
        &plan.build,
        verifier,
    )
    .map_err(DownloadError::Signature)?;
    let dir = paths.downloads_dir();
    std::fs::create_dir_all(&dir).map_err(|e| DownloadError::Io(e.to_string()))?;
    let name = format!(
        "{}-{}-{}",
        plan.plugin_id,
        plan.version,
        &plan.build.fingerprint[..8]
    );
    let part = dir.join(format!("{name}.part"));
    let file = dir.join(format!("{name}.orbitplugin"));
    let result = fetch_and_check(plan, &part, &file, progress);
    if result.is_err() {
        let _ = std::fs::remove_file(&part);
        let _ = std::fs::remove_file(&file);
    }
    result.map(|()| Verified {
        file,
        signature,
        trust,
    })
}

fn fetch_and_check(
    plan: &DownloadPlan,
    part: &std::path::Path,
    file: &std::path::Path,
    progress: &dyn Fn(u64, u64) -> bool,
) -> Result<(), DownloadError> {
    let expected = plan.build.size;
    let url = orbit_http::resolve_url(&plan.source.url, &plan.build.url);
    let timeout = Duration::from_secs(600);
    let done = orbit_http::Request::get(&url, timeout)
        .download(part, expected, &|got, _| progress(got, expected))
        .map_err(|e| match e {
            orbit_http::DownloadError::Cancelled => DownloadError::Cancelled,
            orbit_http::DownloadError::TooLarge => DownloadError::Size {
                expected,
                got: expected + 1,
            },
            orbit_http::DownloadError::Failed(e) => DownloadError::Network(e),
        })?;
    if !(200..300).contains(&done.status) {
        return Err(DownloadError::Http(done.status));
    }
    if done.size != expected {
        return Err(DownloadError::Size {
            expected,
            got: done.size,
        });
    }
    let (got, _) = orbit_sign::sha256::file_hex(part, &|| false)
        .map_err(|e| DownloadError::Io(e.to_string()))?
        .unwrap_or_default();
    if got != plan.build.sha256 {
        return Err(DownloadError::Checksum {
            expected: plan.build.sha256.clone(),
            got,
        });
    }
    std::fs::rename(part, file).map_err(|e| DownloadError::Io(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signing::{Ed25519, public_key_hex, sign_build};
    use crate::sources::Sources;
    use crate::test_util::scratch;
    use orbit_http::testing::{Answer, Server};

    const FP: &str = "0123456789abcdef0123456789abcdef";

    fn plan(server: &Server, data: &[u8], seed: Option<&[u8; 32]>, key: &str) -> DownloadPlan {
        let sha256 = orbit_sign::sha256::digest_hex(data);
        let mut sources = Sources::default();
        let source = sources
            .add(&server.url("/index.json"), "Mock", key)
            .unwrap();
        DownloadPlan {
            source,
            plugin_id: "acme.hello".into(),
            version: "1.0.0".into(),
            build: Build {
                editor_version: "0.4.0".into(),
                fingerprint: FP.into(),
                target: String::new(),
                profile: "release".into(),
                // Relative to the catalog URL.
                url: "builds/hello.orbitplugin".into(),
                signature: seed.map(|s| sign_build(s, "acme.hello", "1.0.0", FP, &sha256)),
                sha256,
                size: data.len() as u64,
            },
        }
    }

    #[test]
    fn verified_downloads_and_refusals() {
        let dir = scratch("download");
        let paths = StorePaths { root: dir };
        let server = Server::start();
        let data: Vec<u8> = (0..50_000u32).map(|i| (i * 7) as u8).collect();
        server.route("/builds/hello.orbitplugin", Answer::ok(data.clone()));
        let seed = [11u8; 32];
        let key = public_key_hex(&seed);
        let v = Ed25519;
        let all = |_, _| true;

        // Trusted source, valid signature.
        let ok = download(&paths, &plan(&server, &data, Some(&seed), &key), &v, &all).unwrap();
        assert_eq!(std::fs::read(&ok.file).unwrap(), data);
        assert_eq!(
            (ok.trust, ok.signature),
            (Trust::Trusted, SignatureState::Valid)
        );
        assert!(ok.file.extension().is_some_and(|e| e == "orbitplugin"));

        // Unsigned build from a trusted source: refused before downloading.
        let before = server.seen().len();
        let err = download(&paths, &plan(&server, &data, None, &key), &v, &all).unwrap_err();
        assert_eq!(err, DownloadError::Signature(SignatureError::Missing));
        assert_eq!(server.seen().len(), before);
        // Signed by someone else.
        let err = download(
            &paths,
            &plan(&server, &data, Some(&[12u8; 32]), &key),
            &v,
            &all,
        )
        .unwrap_err();
        assert_eq!(err, DownloadError::Signature(SignatureError::Invalid));

        // Unsigned source: accepted on the checksum alone.
        let ok = download(&paths, &plan(&server, &data, None, ""), &v, &all).unwrap();
        assert_eq!(
            (ok.trust, ok.signature),
            (Trust::Unsigned, SignatureState::NotChecked)
        );

        // The file served differs from what was signed: SHA-256 mismatch.
        let mut tampered = data.clone();
        tampered[100] ^= 1;
        server.route("/builds/hello.orbitplugin", Answer::ok(tampered));
        let p = plan(&server, &data, Some(&seed), &key);
        let err = download(&paths, &p, &v, &all).unwrap_err();
        assert!(matches!(err, DownloadError::Checksum { .. }), "{err:?}");
        assert!(
            !paths
                .downloads_dir()
                .join("acme.hello-1.0.0-01234567.orbitplugin")
                .exists()
        );

        // Wrong size, missing file, cancel.
        server.route(
            "/builds/hello.orbitplugin",
            Answer::ok(data[..1000].to_vec()),
        );
        assert!(matches!(
            download(&paths, &p, &v, &all),
            Err(DownloadError::Size { .. })
        ));
        server.route("/builds/hello.orbitplugin", Answer::status(404));
        assert_eq!(
            download(&paths, &p, &v, &all).unwrap_err(),
            DownloadError::Http(404)
        );
        server.route("/builds/hello.orbitplugin", Answer::ok(data.clone()));
        assert_eq!(
            download(&paths, &p, &v, &|_, _| false).unwrap_err(),
            DownloadError::Cancelled
        );
        // Leftovers are cleaned.
        let left: Vec<_> = std::fs::read_dir(paths.downloads_dir())
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert!(
            left.iter().all(|n| !n.to_string_lossy().ends_with(".part")),
            "{left:?}"
        );
    }
}
