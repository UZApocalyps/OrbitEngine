//! How the player was asked to start: command line (native) or URL query
//! (web), reduced to a [`RuntimeConfig`].

use orbit_assets::PakLocation;
use orbit_runtime::RuntimeConfig;

/// Story played when none is given (relative to the workspace / the page).
const DEFAULT_STORY: &str = "examples/stories/demo.ron";

/// Resolved launch request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Launch {
    /// Play the hard-coded visual demo instead of a story.
    pub demo: bool,
    /// Show the M2 UI gallery (every menu screen with fake data).
    pub ui_gallery: bool,
    /// Story file, as given (`None` = the default demo story).
    pub story: Option<String>,
    /// Native: root the manifest's asset paths are relative to.
    pub root: Option<String>,
    /// Native: reload the story when its file changes.
    pub watch: bool,
    /// Skip the main menu: start a new game as soon as everything is loaded.
    pub new_game: bool,
    /// Native: directory for saves, preferences and persistent data instead
    /// of the per-user default.
    pub data_dir: Option<String>,
    /// An exported game: the pak to play (`--pak <file>` natively; the
    /// `ORBIT_PAK` global of the page or `?pak=<url>` on the web). Natively,
    /// a pak glued to the executable or a `game.pak` beside it is found
    /// automatically when nothing else is asked for.
    pub pak: Option<PakLocation>,
}

impl Default for Launch {
    fn default() -> Self {
        Self {
            demo: false,
            ui_gallery: false,
            story: None,
            root: None,
            watch: true,
            new_game: false,
            data_dir: None,
            pak: None,
        }
    }
}

/// Parses the native command line (without the program name):
/// `[story] [--story <file>] [--root <dir>] [--no-watch] [--new-game]
/// [--data-dir <dir>] [--pak <file>] [--demo] [--ui-gallery]`.
pub fn parse_args(args: &[String]) -> Result<Launch, String> {
    let mut launch = Launch::default();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--demo" => launch.demo = true,
            "--ui-gallery" => launch.ui_gallery = true,
            "--no-watch" => launch.watch = false,
            "--new-game" => launch.new_game = true,
            "--data-dir" => {
                launch.data_dir = Some(iter.next().ok_or("--data-dir needs a directory")?.clone());
            }
            "--pak" => {
                launch.pak = Some(PakLocation::file(
                    iter.next().ok_or("--pak needs a file")?.clone(),
                ));
            }
            "--story" => {
                launch.story = Some(iter.next().ok_or("--story needs a file")?.clone());
            }
            "--root" => {
                launch.root = Some(iter.next().ok_or("--root needs a directory")?.clone());
            }
            flag if flag.starts_with("--") => return Err(format!("unknown option `{flag}`")),
            file => {
                if launch.story.is_some() {
                    return Err(format!("unexpected extra argument `{file}`"));
                }
                launch.story = Some(file.to_owned());
            }
        }
    }
    Ok(launch)
}

/// Splits a URL query string (`?a=1&b&c=x%20y`, leading `?` optional) into
/// key/value pairs. A key without `=` has an empty value. Percent escapes and
/// `+` are decoded.
pub fn parse_query(search: &str) -> Vec<(String, String)> {
    search
        .trim_start_matches('?')
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| match pair.split_once('=') {
            Some((k, v)) => (percent_decode(k), percent_decode(v)),
            None => (percent_decode(pair), String::new()),
        })
        .collect()
}

/// First value of `key` in the query, if present.
pub fn query_param(search: &str, key: &str) -> Option<String> {
    parse_query(search)
        .into_iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v)
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if bytes.len() >= i + 3 => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
                match hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                    Some(byte) => {
                        out.push(byte);
                        i += 2;
                    }
                    None => out.push(b'%'),
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

impl Launch {
    /// Reads the request from the environment: process arguments natively,
    /// the page's query string on the web.
    pub fn from_environment() -> Result<Self, String> {
        #[cfg(target_arch = "wasm32")]
        {
            let search = web_sys::window()
                .and_then(|w| w.location().search().ok())
                .unwrap_or_default();
            Ok(Self::from_query(&search))
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let args: Vec<String> = std::env::args().skip(1).collect();
            let mut launch = parse_args(&args)?;
            // A game exported with the engine carries its pak: nothing to
            // pass on the command line.
            let dev_request =
                launch.demo || launch.ui_gallery || launch.story.is_some() || launch.pak.is_some();
            if !dev_request {
                launch.pak = PakLocation::beside_executable();
            }
            Ok(launch)
        }
    }

    /// Web: `?demo`, `?ui-gallery`, `?new-game`, `?story=<path>` and
    /// `?pak=<url>`; the page of an exported game sets the `ORBIT_PAK` global
    /// instead.
    pub fn from_query(search: &str) -> Self {
        let query = parse_query(search);
        Self {
            pak: query_param(search, "pak")
                .filter(|p| !p.is_empty())
                .or_else(page_pak)
                .map(PakLocation::url),
            demo: query.iter().any(|(k, _)| k == "demo"),
            ui_gallery: query.iter().any(|(k, _)| k == "ui-gallery"),
            story: query_param(search, "story").filter(|s| !s.is_empty()),
            root: None,
            watch: false,
            new_game: query.iter().any(|(k, _)| k == "new-game"),
            data_dir: None,
        }
    }

    /// The runtime configuration for this launch.
    pub fn runtime_config(&self) -> RuntimeConfig {
        #[cfg(target_arch = "wasm32")]
        let mut config = {
            let story = self.story.clone().unwrap_or_else(|| DEFAULT_STORY.into());
            let mut config = RuntimeConfig::new(".", story, ".", ".");
            // The assets of an exported web game arrive scene by scene.
            config.lazy_assets = self.pak.is_some();
            config
        };
        #[cfg(not(target_arch = "wasm32"))]
        let mut config = native_config(self, std::path::Path::new(env!("CARGO_MANIFEST_DIR")));
        config.auto_start = self.new_game;
        config.data_dir = self.data_dir.clone();
        if let Some(pak) = &self.pak {
            config.pak = Some(pak.clone());
            config.watch_path = None;
        }
        config
    }
}

/// Web: the pak announced by the page (`window.ORBIT_PAK = "game.pak"`).
#[cfg(target_arch = "wasm32")]
fn page_pak() -> Option<String> {
    let window = web_sys::window()?;
    js_sys::Reflect::get(&window, &"ORBIT_PAK".into())
        .ok()?
        .as_string()
        .filter(|p| !p.is_empty())
}

#[cfg(not(target_arch = "wasm32"))]
fn page_pak() -> Option<String> {
    None
}

/// Native configuration. `manifest_dir` is this crate's directory: the
/// workspace root (default story, fonts) is two levels up.
///
/// Asset root, in order: `--root`; the directory itself when the story
/// argument is a directory (a project holding `story.orbit` or legacy `story.ron`); the workspace for
/// the default story; else the current directory (the same rule as
/// `orbit validate --check-files`).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn native_config(launch: &Launch, manifest_dir: &std::path::Path) -> RuntimeConfig {
    use std::path::{Path, PathBuf};

    let workspace = manifest_dir.join("..").join("..");
    let (story_file, default_root): (PathBuf, PathBuf) = match launch.story.as_deref() {
        None => (workspace.join(DEFAULT_STORY), workspace.clone()),
        Some(arg) => {
            let path = Path::new(arg);
            if path.is_dir() {
                // `story.orbit`, else the legacy `story.ron`.
                let name =
                    if path.join("story.orbit").is_file() || !path.join("story.ron").is_file() {
                        "story.orbit"
                    } else {
                        "story.ron"
                    };
                (path.join(name), path.to_path_buf())
            } else {
                (path.to_path_buf(), PathBuf::from("."))
            }
        }
    };
    let root = launch.root.as_deref().map_or(default_root, PathBuf::from);
    let dir = match story_file.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let file_name = story_file.file_name().map_or_else(
        || "story.orbit".to_owned(),
        |n| n.to_string_lossy().into_owned(),
    );
    let mut config = RuntimeConfig::new(
        dir.to_string_lossy(),
        file_name,
        root.to_string_lossy(),
        workspace.to_string_lossy(),
    );
    if launch.watch {
        config.watch_path = Some(story_file.to_string_lossy().into_owned());
    }
    // A project folder brings its own screens (`ui/`).
    let ui_dir = dir.join("ui");
    if launch.story.is_some() && ui_dir.is_dir() {
        config.ui_dir = Some(ui_dir.to_string_lossy().into_owned());
    }
    config
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn no_arguments_means_default_story_with_watching() {
        let l = parse_args(&[]).unwrap();
        assert_eq!(l, Launch::default());
        assert!(l.watch && !l.demo && l.story.is_none());
    }

    #[test]
    fn story_can_be_positional_or_flagged() {
        assert_eq!(
            parse_args(&args(&["a.ron"])).unwrap().story.as_deref(),
            Some("a.ron")
        );
        assert_eq!(
            parse_args(&args(&["--story", "b.ron"]))
                .unwrap()
                .story
                .as_deref(),
            Some("b.ron")
        );
    }

    #[test]
    fn flags_are_parsed() {
        let l = parse_args(&args(&["--demo", "--no-watch", "--root", "proj"])).unwrap();
        assert!(l.demo && !l.watch);
        let l2 = parse_args(&args(&["--new-game", "--data-dir", "saves"])).unwrap();
        assert!(l2.new_game);
        assert_eq!(l2.data_dir.as_deref(), Some("saves"));
        assert!(Launch::from_query("?new-game").new_game);
        assert!(parse_args(&args(&["--data-dir"])).is_err());
        assert!(parse_args(&args(&["--ui-gallery"])).unwrap().ui_gallery);
        let with_pak = parse_args(&args(&["--pak", "g.pak"])).unwrap();
        assert_eq!(with_pak.pak, Some(PakLocation::file("g.pak")));
        assert!(parse_args(&args(&["--pak"])).is_err());
        assert_eq!(
            Launch::from_query("?pak=game.pak").pak,
            Some(PakLocation::url("game.pak"))
        );
        assert!(Launch::from_query("?story=a.ron").pak.is_none());
        assert!(Launch::from_query("?ui-gallery").ui_gallery);
        assert!(!Launch::from_query("?demo").ui_gallery);
        assert_eq!(l.root.as_deref(), Some("proj"));
    }

    #[test]
    fn bad_command_lines_are_errors() {
        assert!(parse_args(&args(&["--story"])).is_err());
        assert!(parse_args(&args(&["--root"])).is_err());
        assert!(parse_args(&args(&["--bogus"])).is_err());
        assert!(parse_args(&args(&["a.ron", "b.ron"])).is_err());
    }

    #[test]
    fn query_parsing() {
        assert_eq!(
            query_param("?story=examples/stories/demo.ron", "story").as_deref(),
            Some("examples/stories/demo.ron")
        );
        assert_eq!(query_param("?demo", "demo").as_deref(), Some(""));
        assert_eq!(query_param("?a=1", "b"), None);
        assert_eq!(query_param("", "b"), None);
        assert_eq!(
            query_param("?story=a%20b+c.ron&x=1", "story").as_deref(),
            Some("a b c.ron")
        );
        // Truncated / invalid escapes stay literal.
        assert_eq!(query_param("?s=100%", "s").as_deref(), Some("100%"));
        assert_eq!(query_param("?s=%zz", "s").as_deref(), Some("%zz"));
    }

    #[test]
    fn a_story_path_containing_demo_is_not_the_demo_flag() {
        let l = Launch::from_query("?story=examples/stories/demo.ron");
        assert!(!l.demo);
        assert_eq!(l.story.as_deref(), Some("examples/stories/demo.ron"));
        assert!(Launch::from_query("?demo").demo);
        assert!(Launch::from_query("?foo=1&demo").demo);
        assert!(!l.watch);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_default_story_uses_the_workspace_as_root() {
        let manifest = std::path::Path::new("/ws/crates/orbit_player");
        let c = native_config(&Launch::default(), manifest);
        assert!(c.story_path == "demo.ron");
        assert!(
            c.story_root
                .replace('\\', "/")
                .ends_with("examples/stories")
        );
        assert!(
            c.assets_root
                .replace('\\', "/")
                .ends_with("crates/orbit_player/../..")
        );
        assert!(c.watch_path.is_some());
        assert!(
            c.engine_root
                .replace('\\', "/")
                .ends_with("crates/orbit_player/../..")
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_explicit_story_uses_cwd_or_root_and_can_disable_watch() {
        let manifest = std::path::Path::new("/ws/crates/orbit_player");
        let launch = Launch {
            story: Some("my/story.ron".into()),
            watch: false,
            ..Default::default()
        };
        let c = native_config(&launch, manifest);
        assert_eq!(c.story_path, "story.ron");
        assert_eq!(c.story_root.replace('\\', "/"), "my");
        assert_eq!(c.assets_root, ".");
        assert!(c.watch_path.is_none());
        let launch = Launch {
            root: Some("/proj".into()),
            ..launch
        };
        assert_eq!(native_config(&launch, manifest).assets_root, "/proj");
        let bare = Launch {
            story: Some("s.ron".into()),
            ..Default::default()
        };
        assert_eq!(native_config(&bare, manifest).story_root, ".");
    }
}
