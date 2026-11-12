//! Native file dialogs for plugins.
//!
//! The editor owns the dialog library (`rfd`); a plugin asks through
//! [`EditorContext::pick_folder`](crate::EditorContext::pick_folder),
//! `pick_file` and `save_file`. Only `std` types cross the plugin boundary.
//! The dialog blocks the editor thread, as the editor's own dialogs do, and
//! the editor tells MCP clients it is busy while it is open.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// A named group of extensions (without the dot) in a file dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileFilter {
    pub name: String,
    pub extensions: Vec<String>,
}

impl FileFilter {
    pub fn new(name: &str, extensions: &[&str]) -> Self {
        Self {
            name: name.to_owned(),
            extensions: extensions.iter().map(|e| (*e).to_owned()).collect(),
        }
    }
}

/// What a dialog shows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileDialogRequest {
    pub title: String,
    /// Folder the dialog starts in.
    pub directory: Option<PathBuf>,
    /// Suggested name (save dialogs).
    pub file_name: Option<String>,
    pub filters: Vec<FileFilter>,
}

/// What shows the dialogs; installed by the editor.
pub trait FileDialogProvider: Send + Sync {
    fn pick_folder(&self, request: &FileDialogRequest) -> Option<PathBuf>;
    fn open_file(&self, request: &FileDialogRequest) -> Option<PathBuf>;
    fn save_file(&self, request: &FileDialogRequest) -> Option<PathBuf>;
}

/// The provider shared by the editor state and its clones. Until the editor
/// installs one (and in tests) every dialog answers `None`, as if cancelled.
#[derive(Clone, Default)]
pub struct FileDialogs(Arc<Mutex<Option<Arc<dyn FileDialogProvider>>>>);

impl FileDialogs {
    pub fn set(&self, provider: Arc<dyn FileDialogProvider>) {
        if let Ok(mut slot) = self.0.lock() {
            *slot = Some(provider);
        }
    }

    fn provider(&self) -> Option<Arc<dyn FileDialogProvider>> {
        // Released before the dialog opens: it can take minutes.
        self.0.lock().ok().and_then(|slot| slot.clone())
    }

    pub fn pick_folder(&self, request: &FileDialogRequest) -> Option<PathBuf> {
        self.provider()?.pick_folder(request)
    }

    pub fn open_file(&self, request: &FileDialogRequest) -> Option<PathBuf> {
        self.provider()?.open_file(request)
    }

    pub fn save_file(&self, request: &FileDialogRequest) -> Option<PathBuf> {
        self.provider()?.save_file(request)
    }
}

impl std::fmt::Debug for FileDialogs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FileDialogs")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as M;

    #[derive(Default)]
    struct Fake(M<Vec<(&'static str, FileDialogRequest)>>);

    impl FileDialogProvider for Fake {
        fn pick_folder(&self, r: &FileDialogRequest) -> Option<PathBuf> {
            self.0.lock().unwrap().push(("folder", r.clone()));
            Some(PathBuf::from("dir"))
        }
        fn open_file(&self, r: &FileDialogRequest) -> Option<PathBuf> {
            self.0.lock().unwrap().push(("open", r.clone()));
            None
        }
        fn save_file(&self, r: &FileDialogRequest) -> Option<PathBuf> {
            self.0.lock().unwrap().push(("save", r.clone()));
            Some(PathBuf::from("out.csv"))
        }
    }

    #[test]
    fn without_a_provider_every_dialog_is_cancelled() {
        let dialogs = FileDialogs::default();
        let request = FileDialogRequest::default();
        assert_eq!(dialogs.pick_folder(&request), None);
        assert_eq!(dialogs.open_file(&request), None);
        assert_eq!(dialogs.save_file(&request), None);
    }

    #[test]
    fn requests_reach_the_provider_through_every_clone() {
        let dialogs = FileDialogs::default();
        let clone = dialogs.clone();
        let fake = Arc::new(Fake::default());
        dialogs.set(fake.clone());
        let request = FileDialogRequest {
            title: "Export".into(),
            directory: Some(PathBuf::from("root")),
            file_name: Some("a.csv".into()),
            filters: vec![FileFilter::new("CSV", &["csv"])],
        };
        assert_eq!(clone.save_file(&request), Some(PathBuf::from("out.csv")));
        assert_eq!(clone.pick_folder(&request), Some(PathBuf::from("dir")));
        assert_eq!(clone.open_file(&request), None);
        let seen = fake.0.lock().unwrap();
        assert_eq!(seen.len(), 3);
        assert_eq!(seen[0], ("save", request.clone()));
        assert_eq!(seen[0].1.filters[0].extensions, ["csv"]);
    }
}
