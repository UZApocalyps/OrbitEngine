//! Path confinement: the server only ever touches files inside the project
//! folder.
//!
//! Every path coming from a client goes through [`Sandbox::resolve`], which
//! rejects `..`, resolves symbolic links (and junctions) and refuses anything
//! whose real location is outside the project root, including dangling links
//! that a write would follow out of the folder.

use std::fs;
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};

use orbit_project::ProjectStorage;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SandboxError {
    #[error("invalid path: {0}")]
    Invalid(String),
    #[error("path `{0}` is outside the project folder")]
    Escapes(String),
    #[error("I/O error: {0}")]
    Io(String),
}

/// The project folder, in canonical form.
#[derive(Debug, Clone)]
pub struct Sandbox {
    root: PathBuf,
}

impl Sandbox {
    /// `root` must be an existing directory.
    pub fn new(root: &Path) -> io::Result<Self> {
        let root = fs::canonicalize(root)?;
        if !root.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "project root is not a directory",
            ));
        }
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Resolves a path given by a client (relative to the project root, or an
    /// absolute path that lies inside it) to a canonical path inside the
    /// project. The target does not need to exist.
    pub fn resolve(&self, input: &str) -> Result<PathBuf, SandboxError> {
        if input.is_empty() || input.contains('\0') {
            return Err(SandboxError::Invalid(
                "empty path or NUL character".to_owned(),
            ));
        }
        let path = Path::new(input);
        let mut current = if path.is_absolute() {
            PathBuf::new()
        } else {
            self.root.clone()
        };
        for component in path.components() {
            match component {
                Component::ParentDir => return Err(SandboxError::Escapes(input.to_owned())),
                Component::CurDir => {}
                Component::Prefix(_) | Component::RootDir => current.push(component),
                Component::Normal(name) => {
                    current.push(name);
                    match fs::symlink_metadata(&current) {
                        Ok(meta) if meta.file_type().is_symlink() => {
                            // Follow the link now; a dangling one is refused
                            // because a write would create its target.
                            current = fs::canonicalize(&current)
                                .map_err(|_| SandboxError::Escapes(input.to_owned()))?;
                        }
                        Ok(_) => {}
                        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                        Err(e) => return Err(SandboxError::Io(e.to_string())),
                    }
                }
            }
        }
        // Canonicalize the deepest existing ancestor and re-append the rest.
        let mut existing = current.clone();
        let mut rest = Vec::new();
        while !existing.exists() {
            match (existing.file_name(), existing.parent()) {
                (Some(name), Some(parent)) => {
                    rest.push(name.to_owned());
                    existing = parent.to_owned();
                }
                _ => return Err(SandboxError::Escapes(input.to_owned())),
            }
        }
        let mut canonical =
            fs::canonicalize(&existing).map_err(|e| SandboxError::Io(e.to_string()))?;
        for name in rest.into_iter().rev() {
            canonical.push(name);
        }
        if canonical.starts_with(&self.root) {
            Ok(canonical)
        } else {
            Err(SandboxError::Escapes(input.to_owned()))
        }
    }

    /// Path of `absolute` relative to the root, with `/` separators.
    pub fn relative(&self, absolute: &Path) -> Option<String> {
        let rel = absolute.strip_prefix(&self.root).ok()?;
        Some(
            rel.components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/"),
        )
    }
}

/// [`ProjectStorage`] confined to the sandbox, with atomic writes (temporary
/// file in the same folder, flush, rename over the target).
#[derive(Debug, Clone)]
pub struct FsStorage {
    sandbox: Sandbox,
}

impl FsStorage {
    pub fn new(sandbox: Sandbox) -> Self {
        Self { sandbox }
    }

    fn resolve(&self, rel: &str) -> io::Result<PathBuf> {
        self.sandbox
            .resolve(rel)
            .map_err(|e| io::Error::new(io::ErrorKind::PermissionDenied, e.to_string()))
    }
}

impl ProjectStorage for FsStorage {
    fn read_to_string(&self, path: &str) -> io::Result<String> {
        fs::read_to_string(self.resolve(path)?)
    }

    fn write_atomic(&self, path: &str, contents: &str) -> io::Result<()> {
        let target = self.resolve(path)?;
        let dir = target
            .parent()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no parent folder"))?;
        fs::create_dir_all(dir)?;
        let name = target
            .file_name()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no file name"))?;
        let tmp = dir.join(format!(".{}.tmp", name.to_string_lossy()));
        let result = (|| {
            let mut file = fs::File::create(&tmp)?;
            file.write_all(contents.as_bytes())?;
            file.sync_all()?;
            drop(file);
            fs::rename(&tmp, &target)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        result
    }

    fn exists(&self, path: &str) -> bool {
        self.resolve(path).is_ok_and(|p| p.exists())
    }

    fn remove(&self, path: &str) -> io::Result<()> {
        match fs::remove_file(self.resolve(path)?) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "orbit_mcp_sandbox_{tag}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn relative_paths_inside_the_project_resolve() {
        let dir = temp_dir("ok");
        let sb = Sandbox::new(&dir).unwrap();
        let p = sb.resolve("assets/bg/room.png").unwrap();
        assert!(p.starts_with(sb.root()));
        assert_eq!(sb.relative(&p).as_deref(), Some("assets/bg/room.png"));
        let abs = sb.root().join("story.ron");
        assert!(sb.resolve(&abs.to_string_lossy()).is_ok());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn escapes_are_refused() {
        let dir = temp_dir("esc");
        let sb = Sandbox::new(&dir).unwrap();
        assert!(sb.resolve("../x").is_err());
        assert!(sb.resolve("a/../../x").is_err());
        assert!(sb.resolve("a/../b").is_err(), "any `..` is refused");
        assert!(sb.resolve("").is_err());
        let outside = std::env::temp_dir().join("orbit_mcp_outside.txt");
        assert!(matches!(
            sb.resolve(&outside.to_string_lossy()),
            Err(SandboxError::Escapes(_))
        ));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn symlinks_out_of_the_project_are_refused() {
        let dir = temp_dir("link");
        let outside = temp_dir("link_outside");
        fs::write(outside.join("secret.txt"), "x").unwrap();
        let link = dir.join("escape");
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(&outside, &link).is_ok();
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_dir(&outside, &link).is_ok();
        if made {
            let sb = Sandbox::new(&dir).unwrap();
            assert!(sb.resolve("escape/secret.txt").is_err());
            assert!(sb.resolve("escape/new.txt").is_err());
        }
        fs::remove_dir_all(&dir).unwrap();
        fs::remove_dir_all(&outside).unwrap();
    }

    #[test]
    fn storage_writes_atomically_inside_the_project_only() {
        let dir = temp_dir("store");
        let storage = FsStorage::new(Sandbox::new(&dir).unwrap());
        storage.write_atomic("sub/a.txt", "hello").unwrap();
        assert_eq!(storage.read_to_string("sub/a.txt").unwrap(), "hello");
        assert!(storage.write_atomic("../evil.txt", "no").is_err());
        assert!(storage.read_to_string("../evil.txt").is_err());
        assert!(!dir.join("sub/.a.txt.tmp").exists());
        fs::remove_dir_all(&dir).unwrap();
    }
}
