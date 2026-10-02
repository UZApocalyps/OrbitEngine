//! Workspace automation. Run through the cargo alias: `cargo xtask <command>`.

mod arch;
mod assets;
mod serve;
mod web;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

const HELP: &str = "\
cargo xtask <command>

Commands:
  web           Build orbit_player for wasm32 (release), bundle it + assets/fonts + example stories into target/web/
  serve         Serve target/web/ on http://127.0.0.1:8080, or --port <n> (blocks until Ctrl+C)
  check-arch    Verify core crates have no GPU/window deps and compile to wasm32
  gen-assets    Regenerate assets/logo.png and the procedural assets/demo/ files
";

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let cmd = args.next();
    let rest: Vec<String> = args.collect();
    let result = match cmd.as_deref() {
        Some("web") => web::build(),
        Some("serve") => serve::serve(&rest),
        Some("check-arch") => arch::check(),
        Some("gen-assets") => assets::generate(),
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
