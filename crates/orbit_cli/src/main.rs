use std::io::{self, BufReader};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let stdin = io::stdin();
    let mut input = BufReader::new(stdin.lock());
    let code = orbit_cli::run(&args, &mut input, &mut io::stdout(), &mut io::stderr());
    ExitCode::from(code)
}
