//! The native file dialogs offered to plugins (`EditorContext::pick_folder`,
//! `pick_file`, `save_file`): `rfd`, like the editor's own dialogs, with the
//! MCP server told the editor is busy while one is open.

use std::path::PathBuf;

use orbit_editor_api::dialogs::{FileDialogProvider, FileDialogRequest};
use orbit_editor_api::mcp::McpQueue;

pub struct RfdDialogs {
    queue: McpQueue,
}

impl RfdDialogs {
    pub fn new(queue: McpQueue) -> Self {
        Self { queue }
    }

    fn dialog(request: &FileDialogRequest) -> rfd::FileDialog {
        let mut dialog = rfd::FileDialog::new().set_title(request.title.as_str());
        if let Some(dir) = &request.directory {
            dialog = dialog.set_directory(dir);
        }
        if let Some(name) = &request.file_name {
            dialog = dialog.set_file_name(name.as_str());
        }
        for filter in &request.filters {
            dialog = dialog.add_filter(filter.name.as_str(), &filter.extensions);
        }
        dialog
    }

    /// Runs `show` with the MCP server marked busy.
    fn busy<T>(&self, request: &FileDialogRequest, show: impl FnOnce(rfd::FileDialog) -> T) -> T {
        self.queue
            .set_busy(Some(format!("the \"{}\" file dialog is open", request.title)));
        let picked = show(Self::dialog(request));
        self.queue.set_busy(None);
        picked
    }
}

impl FileDialogProvider for RfdDialogs {
    fn pick_folder(&self, request: &FileDialogRequest) -> Option<PathBuf> {
        self.busy(request, |d| d.pick_folder())
    }

    fn open_file(&self, request: &FileDialogRequest) -> Option<PathBuf> {
        self.busy(request, |d| d.pick_file())
    }

    fn save_file(&self, request: &FileDialogRequest) -> Option<PathBuf> {
        self.busy(request, |d| d.save_file())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_server_is_busy_only_while_a_dialog_is_open() {
        let queue = McpQueue::new();
        let dialogs = RfdDialogs::new(queue.clone());
        let request = FileDialogRequest { title: "Pick".into(), ..FileDialogRequest::default() };
        let reason = dialogs.busy(&request, |_| queue.busy_reason());
        assert_eq!(reason.as_deref(), Some("the \"Pick\" file dialog is open"));
        assert_eq!(queue.busy_reason(), None);
    }
}
