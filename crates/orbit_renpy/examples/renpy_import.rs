//! Imports a Ren'Py game from the command line and prints the report
//! summary (development tool: measure the coverage of the importer on a
//! real game).
//!
//! `cargo run -p orbit_renpy --example renpy_import -- <renpy game> <new project>`

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [src, out] = args.as_slice() else {
        eprintln!("usage: renpy_import <renpy game folder> <new project folder>");
        std::process::exit(2);
    };
    match orbit_renpy::import(src.as_ref(), out.as_ref(), &|_, _| {}) {
        Ok(report) => {
            println!("{}", report.summary());
            println!(
                "validation: {} errors, {} warnings",
                report.validation_errors.len(),
                report.validation_warnings
            );
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}
