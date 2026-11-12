//! Workspace automation. Run through the cargo alias: `cargo xtask <command>`.

mod arch;
mod assets;
mod bench;
mod plugin_store;
mod plugins;
mod serve;
mod templates;
mod web;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

const HELP: &str = "\
cargo xtask <command>

Commands:
  web           Build orbit_player for wasm32 (release), bundle it + assets/fonts + example stories into target/web/
  serve         Serve target/web/ (or --dir <folder>) on http://127.0.0.1:8080, or --port <n> (blocks until Ctrl+C)
  templates     Build the player templates games are exported with (host OS player, wasm player with wasm-opt)
                into target/templates/ (web/ 2D player, webscript/ 2D player with scripts, web3d/ 3D + scripts).
                Options: --out <dir> --no-native --no-web --no-web3d --no-webscript --target <triple>
                --wasm-opt <path> --skip-wasm-opt --check-budget
                --no-android --android-dex-input <aar|jar> (repeatable)
                Android (arm64-v8a, + armeabi-v7a / x86_64 when their rust target is installed) is built
                when cargo-ndk, the NDK and the rust targets are present, and skipped with a message otherwise.
  wasm-budget   Fail if target/templates/web/orbit_player_bg.wasm is over the size budget (--file <wasm>);
                --3d checks target/templates/web3d/ against the 3D player budget, --script webscript/
  check-arch    Verify core crates have no GPU/window deps and compile to wasm32
  bench         Headless reference benchmarks (orbit-editor --bench): writes target/bench/results.json and
                compares the medians with xtask/bench-baseline.json (informational).
                Options: --debug --quick --out <json> --baseline <json> --threshold <ratio, 1.5>
                --strict (fail on a regression) --update-baseline
  gen-assets    Regenerate assets/logo.png and the procedural assets/demo/ files
  plugins       Build the official native editor plugins (plugins/*) and package them as .orbitplugin into
                <target>/plugins/. Options: --debug (default release, same profile as the editor)
                --examples (also examples/editor_plugins/*) --only <package> --out <dir> --no-check
                --sign <keyfile> | --sign-env <VAR> (signed build descriptors for the plugin store)
  plugin-key    gen [--out <file>]: new Ed25519 plugin signing key, written outside any repository;
                pub <file>: print its public key
  plugin-index  Merge the build descriptors of <target>/plugins into a store catalog:
                --index <index.json> [--builds <dir>] [--url <pattern>] [--tag <tag>] [--changelog <text>]
";

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let cmd = args.next();
    let rest: Vec<String> = args.collect();
    let result = match cmd.as_deref() {
        Some("web") => web::build(),
        Some("serve") => serve::serve(&rest),
        Some("templates") => templates::templates(&rest),
        Some("wasm-budget") => templates::budget(&rest),
        Some("check-arch") => arch::check(),
        Some("bench") => bench::bench(&rest),
        Some("gen-assets") => assets::generate(),
        Some("plugins") => plugins::plugins(&rest),
        Some("plugin-key") => plugin_store::plugin_key(&rest),
        Some("plugin-index") => plugin_store::plugin_index(&rest, default_plugins_dir()),
        Some("help") | Some("--help") | Some("-h") | None => {
            print!("{HELP}");
            Ok(())
        }
        Some(other) => Err(format!("unknown command `{other}`\n\n{HELP}").into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

/// `<target>/plugins` (where `cargo xtask plugins` writes the packages).
fn default_plugins_dir() -> PathBuf {
    let target = std::env::var_os("CARGO_TARGET_DIR").map_or_else(|| workspace_root().join("target"), PathBuf::from);
    target.join("plugins")
}

/// Workspace root (the xtask crate lives in `<root>/xtask`).
pub fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask has a parent directory")
        .to_path_buf()
}

pub fn cargo() -> Command {
    let mut cmd = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    cmd.current_dir(workspace_root());
    cmd
}

/// Runs a command, failing with a readable message if it does not succeed.
pub fn run(cmd: &mut Command) -> Result {
    println!("> {cmd:?}");
    let status = cmd.status()?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("command failed with {status}: {cmd:?}").into())
    }
}
