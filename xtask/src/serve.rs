//! `cargo xtask serve`: tiny static HTTP server for `target/web/`.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};
use std::thread;

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

pub fn serve(args: &[String]) -> Result {
    let addr = format!("127.0.0.1:{}", parse_port(args)?);
    let root = workspace_root().join("target/web");
    if !root.join("index.html").exists() {
        return Err("target/web/index.html not found; run `cargo xtask web` first".into());
    }
    let listener = TcpListener::bind(&addr)?;
    println!(
        "Serving {} on http://{addr} (Ctrl+C to stop)",
        root.display()
    );
    for stream in listener.incoming().flatten() {
        let root = root.clone();
        thread::spawn(move || {
            if let Err(err) = handle(stream, &root) {
                eprintln!("connection error: {err}");
            }
        });
    }
    Ok(())
}

pub fn mime_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "wasm" => "application/wasm",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "png" => "image/png",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "jpg" | "jpeg" => "image/jpeg",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "txt" | "ron" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// Maps a request target to a file below `root`, rejecting traversal.
pub fn resolve(root: &Path, target: &str) -> Option<PathBuf> {
    let path = target.split(['?', '#']).next().unwrap_or("");
    let path = path.trim_start_matches('/');
    let mut out = root.to_path_buf();
    if path.is_empty() {
        out.push("index.html");
        return Some(out);
    }
    for comp in Path::new(path).components() {
        match comp {
            Component::Normal(c) => out.push(c),
            _ => return None, // `..`, absolute paths, drive prefixes
        }
    }
    if out.is_dir() {
        out.push("index.html");
    }
    Some(out)
}

fn respond(
    stream: &mut TcpStream,
    status: &str,
    ctype: &str,
    body: &[u8],
    head_only: bool,
) -> Result {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    if !head_only {
        stream.write_all(body)?;
    }
    Ok(())
}

fn handle(mut stream: TcpStream, root: &Path) -> Result {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    // Drain headers.
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 || line == "\r\n" || line == "\n" {
            break;
        }
    }

    let mut parts = request_line.split_whitespace();
    let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or("/"));
    let head_only = method == "HEAD";
    if method != "GET" && !head_only {
        return respond(
            &mut stream,
            "405 Method Not Allowed",
            "text/plain",
            b"method not allowed",
            false,
        );
    }
    match resolve(root, target) {
        None => respond(
            &mut stream,
            "400 Bad Request",
            "text/plain",
            b"bad path",
            head_only,
        ),
        Some(path) => match fs::read(&path) {
            Ok(body) => respond(&mut stream, "200 OK", mime_type(&path), &body, head_only),
            Err(_) => respond(
                &mut stream,
                "404 Not Found",
                "text/plain",
                b"not found",
                head_only,
            ),
        },
    }
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
    fn mime_types() {
        assert_eq!(mime_type(Path::new("a.wasm")), "application/wasm");
        assert!(mime_type(Path::new("a.js")).starts_with("text/javascript"));
        assert!(mime_type(Path::new("a.html")).starts_with("text/html"));
        assert_eq!(mime_type(Path::new("a.wav")), "audio/wav");
        assert_eq!(mime_type(Path::new("a.ttf")), "font/ttf");
    }

    #[test]
    fn resolve_blocks_traversal() {
        let root = Path::new("root");
        assert_eq!(resolve(root, "/"), Some(root.join("index.html")));
        assert_eq!(
            resolve(root, "/a/b.js?x=1"),
            Some(root.join("a").join("b.js"))
        );
        assert_eq!(resolve(root, "/../secret"), None);
        assert_eq!(resolve(root, "/a/../../secret"), None);
    }
}
