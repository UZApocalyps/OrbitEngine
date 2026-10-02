use std::io::{self, BufReader};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Not `stdin.lock()`: a lock held for the whole run would block the MCP
    // server, which reads the process's stdin itself (`orbit mcp`).
    let mut input = BufReader::new(io::stdin());
    let code = orbit_cli::run(&args, &mut input, &mut io::stdout(), &mut io::stderr());
    ExitCode::from(code)
}
