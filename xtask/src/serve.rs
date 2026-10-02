//! `cargo xtask serve`: static HTTP server for a web bundle (the one of
//! `orbit_export`: range requests, precompressed files, `application/wasm`).

use std::path::PathBuf;
use std::sync::Arc;

use orbit_export::Server;

use crate::{Result, workspace_root};

const DEFAULT_PORT: u16 = 8080;

/// Port from `--port <n>` (default 8080).
fn parse_port(args: &[String]) -> Result<u16> {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--port" {
            let value = iter.next().ok_or("--port needs a number")?;
            return value
                .parse()
                .map_err(|_| format!("invalid port `{value}`").into());
        }
    }
    Ok(DEFAULT_PORT)
}

/// The folder given with `--dir <path>`, else `target/web`.
fn parse_dir(args: &[String]) -> Result<PathBuf> {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--dir" {
            return Ok(PathBuf::from(iter.next().ok_or("--dir needs a folder")?));
        }
    }
    Ok(workspace_root().join("target/web"))
}

pub fn serve(args: &[String]) -> Result {
    let root = parse_dir(args)?;
    if !root.join("index.html").exists() {
        return Err(format!(
            "{}/index.html not found; run `cargo xtask web` first",
            root.display()
        )
        .into());
    }
    let log: orbit_export::serve::RequestLog = Arc::new(|line| println!("  {line}"));
    let server = Server::start(root.clone(), Some(parse_port(args)?), Some(log))?;
    println!(
        "Serving {} on {} (Ctrl+C to stop)",
        root.display(),
        server.url()
    );
    server.wait();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_option() {
        let args = |l: &[&str]| l.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(parse_port(&[]).unwrap(), 8080);
        assert_eq!(parse_port(&args(&["--port", "9123"])).unwrap(), 9123);
        assert!(parse_port(&args(&["--port"])).is_err());
        assert!(parse_port(&args(&["--port", "abc"])).is_err());
    }

    #[test]
    fn dir_option() {
        let args = |l: &[&str]| l.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            parse_dir(&args(&["--dir", "x"])).unwrap(),
            PathBuf::from("x")
        );
        assert!(parse_dir(&args(&["--dir"])).is_err());
        assert!(parse_dir(&[]).unwrap().ends_with("target/web"));
    }
}
