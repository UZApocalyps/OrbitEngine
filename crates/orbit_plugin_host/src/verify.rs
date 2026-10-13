//! The checks a plugin library goes through before the editor loads it, and
//! the reasons it can be refused.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use orbit_editor_api::native::{DescriptorInfo, Fingerprint, Mismatch};
use serde::{Deserialize, Serialize};

use crate::binary::BinaryError;
use crate::manifest::{PackageManifest, RangeError};

/// Why a plugin is not loaded. The editor shows it (translated) in the
/// plugin manager; [`std::fmt::Display`] gives the English text (logs).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Problem {
    /// Not a library for this platform / processor.
    Binary(BinaryError),
    /// The probe process failed or crashed while reading the descriptor.
    ProbeFailed(String),
    /// No valid descriptor (not an OrbitEngine plugin, or a too old / too
    /// new descriptor layout).
    BadDescriptor(String),
    /// Built for another editor / compiler / settings.
    Incompatible(Vec<Mismatch>),
    /// The manifest's editor range excludes this editor.
    EditorRange(RangeError),
    /// The manifest and the library disagree on the id.
    IdMismatch { manifest: String, library: String },
    InvalidId(String),
    /// Another plugin (or a built-in tool) already uses this id.
    DuplicateId(String),
    /// The library changed since it was installed (checksum).
    Modified,
    /// The editor stopped while loading it last time.
    CrashedAtLoad,
    /// The library could not be loaded or created.
    LoadFailed(String),
    /// The plugin's `on_load` returned an error.
    InitFailed(String),
    /// The plugin panicked; it is disabled.
    Panicked(String),
    /// Its files are missing or unreadable.
    Missing(String),
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Binary(e) => write!(f, "{e}"),
            Self::ProbeFailed(e) => write!(f, "the library could not be inspected: {e}"),
            Self::BadDescriptor(e) => write!(f, "not an OrbitEngine editor plugin: {e}"),
            Self::Incompatible(m) => write!(f, "{}", describe_mismatches(m)),
            Self::EditorRange(RangeError::TooOld { min }) => {
                write!(f, "requires editor {min} or newer")
            }
            Self::EditorRange(RangeError::TooNew { max }) => {
                write!(f, "supports editors up to {max} only")
            }
            Self::EditorRange(RangeError::Invalid(v)) => write!(f, "invalid editor version `{v}`"),
            Self::IdMismatch { manifest, library } => {
                write!(f, "manifest id `{manifest}` but library id `{library}`")
            }
            Self::InvalidId(id) => write!(f, "invalid plugin id `{id}`"),
            Self::DuplicateId(id) => write!(f, "the id `{id}` is already used"),
            Self::Modified => f.write_str("the library changed since it was installed; reinstall it"),
            Self::CrashedAtLoad => f.write_str("the editor stopped while loading it; disabled"),
            Self::LoadFailed(e) => write!(f, "loading failed: {e}"),
            Self::InitFailed(e) => write!(f, "the plugin failed to start: {e}"),
            Self::Panicked(e) => write!(f, "the plugin crashed and was disabled: {e}"),
            Self::Missing(e) => write!(f, "files missing: {e}"),
        }
    }
}

/// "built for editor 0.3.0 / rustc 1.94.0 (...), this editor is 0.3.1 /
/// rustc 1.95.0 (...): rebuild the plugin" (English; the editor translates).
pub fn describe_mismatches(mismatches: &[Mismatch]) -> String {
    let parts: Vec<String> = mismatches
        .iter()
        .map(|m| format!("{}: plugin {} / editor {}", m.field.key(), m.plugin, m.editor))
        .collect();
    format!("built for another editor build ({}); rebuild the plugin", parts.join("; "))
}

/// What the editor checks plugins against.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorFacts {
    pub fingerprint: Fingerprint,
    /// The editor version (`orbit_editor_api::native::API_VERSION`).
    pub version: String,
}

impl EditorFacts {
    /// This build.
    pub fn current() -> Self {
        Self {
            fingerprint: Fingerprint::current(),
            version: orbit_editor_api::native::API_VERSION.to_owned(),
        }
    }

    /// Key of the cached probe results: a new editor build probes again.
    pub fn cache_key(&self) -> String {
        let f = &self.fingerprint;
        format!(
            "{}|{}|{}|{}|{}|{}|{}|{}",
            f.api_version, f.abi_version, f.rustc, f.egui, f.target, f.profile, f.api_hash, f.layout_hash
        )
    }
}

/// Plugin ids: lowercase dotted names (`acme.hello`).
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.split('.').all(|part| {
            !part.is_empty()
                && part
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        })
}

/// The checks after reading the descriptor: fingerprint, id, manifest.
pub fn verify(
    info: &DescriptorInfo,
    manifest: Option<&PackageManifest>,
    editor: &EditorFacts,
) -> Result<(), Problem> {
    let mismatches = info.fingerprint.mismatches(&editor.fingerprint);
    if !mismatches.is_empty() {
        return Err(Problem::Incompatible(mismatches));
    }
    if !valid_id(&info.id) {
        return Err(Problem::InvalidId(info.id.clone()));
    }
    if let Some(m) = manifest {
        if m.id != info.id {
            return Err(Problem::IdMismatch {
                manifest: m.id.clone(),
                library: info.id.clone(),
            });
        }
        m.check_editor(&editor.version).map_err(Problem::EditorRange)?;
    }
    Ok(())
}

/// Reads a library's descriptor.
pub trait Prober {
    fn probe(&self, library: &Path) -> Result<DescriptorInfo, Problem>;
}

/// What the probe process prints (one JSON line on stdout).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeOutput {
    pub ok: bool,
    #[serde(default)]
    pub info: Option<DescriptorInfo>,
    #[serde(default)]
    pub error: Option<String>,
}

/// Probes in a short-lived child process (`<program> <args...> <library>`),
/// so that a broken library cannot take the editor down: the editor runs
/// itself with `--probe-plugin`.
#[derive(Clone, Debug)]
pub struct ProcessProber {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub timeout: Duration,
}

impl ProcessProber {
    /// `<this executable> --probe-plugin <library>`.
    pub fn current_exe() -> Option<Self> {
        Some(Self {
            program: std::env::current_exe().ok()?,
            args: vec!["--probe-plugin".into()],
            timeout: Duration::from_secs(20),
        })
    }
}

impl Prober for ProcessProber {
    fn probe(&self, library: &Path) -> Result<DescriptorInfo, Problem> {
        let mut command = Command::new(&self.program);
        command
            .args(&self.args)
            .arg(library)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // CREATE_NO_WINDOW: no console flashing up.
            command.creation_flags(0x0800_0000);
        }
        let mut child = command.spawn().map_err(|e| Problem::ProbeFailed(e.to_string()))?;
        let started = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if started.elapsed() > self.timeout => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(Problem::ProbeFailed("timed out".into()));
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(10)),
                Err(e) => return Err(Problem::ProbeFailed(e.to_string())),
            }
        };
        let output = child
            .wait_with_output()
            .map_err(|e| Problem::ProbeFailed(e.to_string()))?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let parsed = stdout
            .lines()
            .rev()
            .find(|l| l.trim_start().starts_with('{'))
            .and_then(|l| serde_json::from_str::<ProbeOutput>(l).ok());
        match parsed {
            Some(ProbeOutput { ok: true, info: Some(info), .. }) => Ok(info),
            Some(ProbeOutput { error, .. }) => Err(Problem::BadDescriptor(error.unwrap_or_default())),
            None => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let tail: String = stderr.lines().rev().take(3).collect::<Vec<_>>().join(" / ");
                Err(Problem::ProbeFailed(format!("{status}{}", if tail.is_empty() { String::new() } else { format!(": {tail}") })))
            }
        }
    }
}

/// Probes in this process (the probe child itself, tests). A library that
/// crashes on load takes the process with it.
#[derive(Clone, Copy, Debug, Default)]
pub struct InProcessProber;

impl Prober for InProcessProber {
    fn probe(&self, library: &Path) -> Result<DescriptorInfo, Problem> {
        crate::binary::check_library_file(library).map_err(Problem::Binary)?;
        crate::load::read_descriptor(library)
    }
}

#[cfg(test)]
mod tests {
    use orbit_editor_api::native::FingerprintField;

    use super::*;

    fn info(id: &str) -> DescriptorInfo {
        DescriptorInfo {
            id: id.into(),
            name: "Hello".into(),
            version: "0.1.0".into(),
            description: String::new(),
            author: String::new(),
            fingerprint: Fingerprint::current(),
        }
    }

    fn manifest(id: &str) -> PackageManifest {
        PackageManifest {
            id: id.into(),
            name: "Hello".into(),
            version: "0.1.0".into(),
            description: String::new(),
            author: String::new(),
            library: "hello.dll".into(),
            min_editor: None,
            max_editor: None,
            homepage: None,
        }
    }

    #[test]
    fn a_matching_plugin_passes() {
        let editor = EditorFacts::current();
        assert_eq!(verify(&info("acme.hello"), Some(&manifest("acme.hello")), &editor), Ok(()));
        assert_eq!(verify(&info("acme.hello"), None, &editor), Ok(()));
    }

    #[test]
    fn each_refusal_has_its_reason() {
        let editor = EditorFacts::current();
        let mut i = info("acme.hello");
        i.fingerprint.rustc = "rustc 1.80.0 (old)".into();
        match verify(&i, None, &editor) {
            Err(Problem::Incompatible(m)) => {
                assert_eq!(m.len(), 1);
                assert_eq!(m[0].field, FingerprintField::Rustc);
                let text = Problem::Incompatible(m).to_string();
                assert!(text.contains("rustc 1.80.0 (old)") && text.contains("rebuild"), "{text}");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            verify(&info("Acme Hello"), None, &editor),
            Err(Problem::InvalidId("Acme Hello".into()))
        );
        assert!(matches!(
            verify(&info("acme.hello"), Some(&manifest("acme.other")), &editor),
            Err(Problem::IdMismatch { .. })
        ));
        let mut m = manifest("acme.hello");
        m.min_editor = Some("999.0.0".into());
        assert!(matches!(
            verify(&info("acme.hello"), Some(&m), &editor),
            Err(Problem::EditorRange(RangeError::TooOld { .. }))
        ));
        let mut m = manifest("acme.hello");
        m.max_editor = Some("0.0.1".into());
        assert!(matches!(
            verify(&info("acme.hello"), Some(&m), &editor),
            Err(Problem::EditorRange(RangeError::TooNew { .. }))
        ));
    }

    #[test]
    fn the_cache_key_follows_the_build() {
        let a = EditorFacts::current();
        let mut b = a.clone();
        assert_eq!(a.cache_key(), b.cache_key());
        b.fingerprint.layout_hash = "0".into();
        assert_ne!(a.cache_key(), b.cache_key());
    }
}
