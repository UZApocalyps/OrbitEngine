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
    if let Some(path) = &args.probe_plugin {
        // Run by the plugin manager in a throw-away process: a broken
        // library can only take this process down.
        let (json, ok) = orbit_plugin_host::probe_main(path);
        println!("{json}");
        return if ok { ExitCode::SUCCESS } else { ExitCode::FAILURE };
    }
    if args.help {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    if let Some((nodes, path)) = &args.generate_story {
        let story = orbit_editor::bench::generate_story(*nodes);
        let written = story
            .to_ron()
            .map_err(|e| e.to_string())
            .and_then(|text| std::fs::write(path, text).map_err(|e| e.to_string()));
        return match written {
            Ok(()) => {
                println!("{} : {nodes} nœuds demandés", path.display());
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("erreur : {e}");
                ExitCode::FAILURE
            }
        };
    }
    if let Some(path) = &args.bench {
        return match orbit_editor::perf::run_to_file(path) {
            Ok(()) => {
                println!("{}", path.display());
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("erreur : {e}");
                ExitCode::FAILURE
            }
        };
    }
    if let Some((blocks, path)) = &args.generate_graph {
        let story = orbit_editor::bench_graph::generate_graph_story(*blocks);
        let written = story
            .to_ron()
            .map_err(|e| e.to_string())
            .and_then(|text| std::fs::write(path, text).map_err(|e| e.to_string()));
        return match written {
            Ok(()) => {
                println!("{} : {blocks} blocs demandés", path.display());
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("erreur : {e}");
                ExitCode::FAILURE
            }
        };
    }
    match orbit_editor::app::run(args, t0) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("erreur : {e}");
            ExitCode::FAILURE
        }
    }
}
