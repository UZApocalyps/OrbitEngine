//! `orbit-plugin-probe <library>`: prints the plugin descriptor as one JSON
//! line (the same as `orbit-editor --probe-plugin <library>`).

use std::process::ExitCode;

fn main() -> ExitCode {
    let Some(path) = std::env::args_os().nth(1) else {
        eprintln!("usage: orbit-plugin-probe <library>");
        return ExitCode::from(2);
    };
    let (json, ok) = orbit_plugin_host::probe_main(std::path::Path::new(&path));
    println!("{json}");
    if ok { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}
