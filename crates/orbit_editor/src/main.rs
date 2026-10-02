//! `orbit-editor`: the OrbitEngine editor (Windows and Linux).

use std::process::ExitCode;
use std::time::Instant;

use orbit_editor::args::{Args, USAGE};

fn main() -> ExitCode {
    // Measured from here: the closest we get to "process start".
    let t0 = Instant::now();
    orbit_editor::logging::init();
    let args = match Args::parse(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("erreur : {message}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    if args.help {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    match orbit_editor::app::run(args, t0) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("erreur : {e}");
            ExitCode::FAILURE
        }
    }
}
