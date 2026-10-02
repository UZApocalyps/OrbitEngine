//! Where the manifest's asset paths are relative to.

use std::path::{Path, PathBuf};

use orbit_core::Story;

/// The folder the manifest's asset paths are relative to. A project folder
/// is its own root; a story file opened alone (the demo) may keep its assets
/// in a parent folder (`assets/demo/...` next to `examples/`): the first
/// ancestor where the first declared asset exists wins.
///
/// Shared by the editor and the MCP server so both check the same files.
pub fn find_assets_root(root: &Path, story: &Story) -> PathBuf {
    let found = story.assets.assets.first().and_then(|first| {
        root.ancestors()
            .take(5)
            .find(|dir| dir.join(&first.path).exists())
    });
    let dir = found.unwrap_or(root);
    // The last ancestor of a relative path is `""`, which `read_dir` refuses.
    if dir.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        dir.to_owned()
    }
}
