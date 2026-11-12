//! Downloading a game plugin package: signature checked first (it covers
//! the catalog's SHA-256), then the file streamed to
//! `<config>/plugin_store/downloads/`, its size and SHA-256 compared with the
//! catalog. Only a file that passes is returned, for [`super::project::install`]
//! (which reads and checks the package itself).

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::PackageSource;
use super::catalog::{GameVersion, check_package};
use super::package::PACKAGE_EXTENSION;
use crate::StorePaths;
use crate::download::DownloadError;
use crate::signing::{SignatureState, SignatureVerifier};

/// What to download: `version` of the game plugin `plugin_id` from `source`.
#[derive(Clone, Debug)]
pub struct GameDownloadPlan {
    pub source: PackageSource,
    pub plugin_id: String,
    pub version: GameVersion,
}

/// A verified package, ready for the installer.
#[derive(Clone, Debug)]
pub struct VerifiedPackage {
    pub file: PathBuf,
    pub signature: SignatureState,
}

/// Downloads and verifies `plan`. `progress(received, total)` returns
/// `false` to cancel. The partial file is removed on any failure.
pub fn download(
    paths: &StorePaths,
    plan: &GameDownloadPlan,
    verifier: &dyn SignatureVerifier,
    progress: &dyn Fn(u64, u64) -> bool,
) -> Result<VerifiedPackage, DownloadError> {
    let signature = check_package(&plan.source, &plan.plugin_id, &plan.version, verifier)
        .map_err(DownloadError::Signature)?;
    let dir = paths.downloads_dir();
    std::fs::create_dir_all(&dir).map_err(|e| DownloadError::Io(e.to_string()))?;
    let name = format!("{}-{}", plan.plugin_id, plan.version.version);
    let part = dir.join(format!("{name}.{PACKAGE_EXTENSION}.part"));
    let file = dir.join(format!("{name}.{PACKAGE_EXTENSION}"));
    let result = fetch_and_check(plan, &part, &file, progress);
    if result.is_err() {
        let _ = std::fs::remove_file(&part);
        let _ = std::fs::remove_file(&file);
    }
    result.map(|()| VerifiedPackage { file, signature })
}

fn fetch_and_check(
    plan: &GameDownloadPlan,
    part: &Path,
    file: &Path,
    progress: &dyn Fn(u64, u64) -> bool,
) -> Result<(), DownloadError> {
    let expected = plan.version.size;
    let url = orbit_http::resolve_url(&plan.source.url, &plan.version.url);
    let done = orbit_http::Request::get(&url, Duration::from_secs(600))
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
    if got != plan.version.sha256 {
        return Err(DownloadError::Checksum {
            expected: plan.version.sha256.clone(),
            got,
        });
    }
    std::fs::rename(part, file).map_err(|e| DownloadError::Io(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::catalog::sign_package;
    use crate::signing::{Ed25519, SignatureError, parse_public_key, public_key_hex};
    use crate::sources::Trust;
    use crate::test_util::scratch;
    use orbit_http::testing::{Answer, Server};

    fn plan(
        server: &Server,
        data: &[u8],
        seed: Option<&[u8; 32]>,
        key: Option<[u8; 32]>,
    ) -> GameDownloadPlan {
        let sha256 = orbit_sign::sha256::digest_hex(data);
        GameDownloadPlan {
            source: PackageSource {
                id: "mock".into(),
                name: "Mock".into(),
                url: server.url("/index.json"),
                trust: if key.is_some() {
                    Trust::Trusted
                } else {
                    Trust::Unsigned
                },
                key,
                official: false,
            },
            plugin_id: "acme.dice".into(),
            version: GameVersion {
                version: "1.0.0".into(),
                released: None,
                changelog: Default::default(),
                api_version: 1,
                min_engine: "0.4.0".into(),
                max_engine: None,
                url: "packages/dice.orbitsrc".into(),
                signature: seed.map(|s| sign_package(s, "acme.dice", "1.0.0", 1, &sha256)),
                sha256,
                size: data.len() as u64,
            },
        }
    }

    #[test]
    fn verified_downloads_and_refusals() {
        let paths = StorePaths {
            root: scratch("game_download"),
        };
        let server = Server::start();
        let data: Vec<u8> = (0..20_000u32).map(|i| (i * 13) as u8).collect();
        server.route("/packages/dice.orbitsrc", Answer::ok(data.clone()));
        let seed = [21u8; 32];
        let key = parse_public_key(&public_key_hex(&seed));
        let all = |_, _| true;

        let ok = download(
            &paths,
            &plan(&server, &data, Some(&seed), key),
            &Ed25519,
            &all,
        )
        .unwrap();
        assert_eq!(std::fs::read(&ok.file).unwrap(), data);
        assert_eq!(ok.signature, SignatureState::Valid);
        assert!(ok.file.extension().is_some_and(|e| e == PACKAGE_EXTENSION));

        // Unsigned package from a trusted source: refused before any request.
        let before = server.seen().len();
        let err = download(&paths, &plan(&server, &data, None, key), &Ed25519, &all).unwrap_err();
        assert_eq!(err, DownloadError::Signature(SignatureError::Missing));
        assert_eq!(server.seen().len(), before);
        // Unsigned source: the checksum only.
        let ok = download(&paths, &plan(&server, &data, None, None), &Ed25519, &all).unwrap();
        assert_eq!(ok.signature, SignatureState::NotChecked);
        // Tampered file.
        let mut tampered = data.clone();
        tampered[7] ^= 1;
        server.route("/packages/dice.orbitsrc", Answer::ok(tampered));
        let err = download(
            &paths,
            &plan(&server, &data, Some(&seed), key),
            &Ed25519,
            &all,
        )
        .unwrap_err();
        assert!(matches!(err, DownloadError::Checksum { .. }), "{err:?}");
        assert!(
            std::fs::read_dir(paths.downloads_dir())
                .unwrap()
                .flatten()
                .all(|e| {
                    let n = e.file_name().to_string_lossy().into_owned();
                    !n.ends_with(".part") && n != "acme.dice-1.0.0.orbitsrc"
                })
        );
    }
}
