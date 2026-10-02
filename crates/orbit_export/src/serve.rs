//! A small static HTTP server to try a web export locally.
//!
//! What a real host does and the player relies on: `Range` requests (the
//! lazy loading of `game.pak`), `application/wasm`, and the precompressed
//! `.br` / `.gz` siblings served for `Accept-Encoding`. It binds to the
//! loopback interface only.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// Called with one line per request served (`GET /game.pak 206 bytes=0-65535`).
pub type RequestLog = Arc<dyn Fn(String) + Send + Sync>;

/// A running server. Stops when dropped.
pub struct Server {
    port: u16,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Server {
    /// Serves `root` on `127.0.0.1`. `port` `None` lets the system pick a
    /// free port (see [`Server::port`]).
    pub fn start(root: PathBuf, port: Option<u16>, log: Option<RequestLog>) -> io::Result<Self> {
        let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port.unwrap_or(0))))?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let thread = thread::Builder::new()
            .name("orbit-export-serve".to_owned())
            .spawn(move || accept_loop(listener, root, flag, log))?;
        Ok(Self {
            port,
            stop,
            thread: Some(thread),
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/", self.port)
    }

    /// Blocks until the server is stopped from another thread (a Ctrl+C
    /// ends the process, which is what the command line wants).
    pub fn wait(mut self) {
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }

    pub fn stop(self) {
        drop(self);
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn accept_loop(listener: TcpListener, root: PathBuf, stop: Arc<AtomicBool>, log: Option<RequestLog>) {
    while !stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, _)) => {
                let root = root.clone();
                let log = log.clone();
                thread::spawn(move || {
                    let _ = stream.set_nonblocking(false);
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
                    if let Err(err) = handle(stream, &root, log.as_deref()) {
                        // A browser closing a connection early is normal.
                        log::debug!("serve: {err}");
                    }
                });
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(15));
            }
            Err(_) => thread::sleep(Duration::from_millis(50)),
        }
    }
}

pub fn mime_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "wasm" => "application/wasm",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "txt" => "text/plain; charset=utf-8",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "ico" => "image/x-icon",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        _ => "application/octet-stream",
    }
}

/// Percent decoding of a URL path.
fn decode(path: &str) -> Option<String> {
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// The file a request path maps to, inside `root` (`None` for anything that
/// could leave it).
pub(crate) fn resolve(root: &Path, target: &str) -> Option<PathBuf> {
    let path = target.split(['?', '#']).next()?;
    let decoded = decode(path)?;
    let mut out = root.to_path_buf();
    for part in decoded.split('/') {
        match part {
            "" | "." => {}
            ".." => return None,
            p if p.contains(['\\', ':']) || p.contains('\0') => return None,
            p => out.push(p),
        }
    }
    if out == root || out.is_dir() {
        out.push("index.html");
    }
    Some(out)
}

/// A parsed `Range: bytes=...` header against a file of `len` bytes.
#[derive(Debug, PartialEq, Eq)]
enum RangeRequest {
    Whole,
    Part(u64, u64),
    Unsatisfiable,
}

fn parse_range(header: Option<&String>, len: u64) -> RangeRequest {
    let Some(spec) = header.and_then(|h| h.trim().strip_prefix("bytes=")) else {
        return RangeRequest::Whole;
    };
    // Only the first range is served.
    let spec = spec.split(',').next().unwrap_or("").trim();
    let Some((a, b)) = spec.split_once('-') else {
        return RangeRequest::Whole;
    };
    let (start, end) = match (a.trim().parse::<u64>(), b.trim().parse::<u64>()) {
        (Ok(a), Ok(b)) => (a, b.min(len.saturating_sub(1))),
        (Ok(a), Err(_)) if b.trim().is_empty() => (a, len.saturating_sub(1)),
        (Err(_), Ok(n)) if a.trim().is_empty() && n > 0 => (len.saturating_sub(n), len.saturating_sub(1)),
        _ => return RangeRequest::Whole,
    };
    if len == 0 || start >= len || start > end {
        RangeRequest::Unsatisfiable
    } else {
        RangeRequest::Part(start, end)
    }
}

fn wants(accept: Option<&String>, coding: &str) -> bool {
    accept.is_some_and(|a| {
        a.split(',')
            .any(|c| c.trim().split(';').next().is_some_and(|n| n.trim() == coding))
    })
}

struct Response {
    status: &'static str,
    headers: Vec<(String, String)>,
    /// What to send: nothing (HEAD, errors), bytes, or a slice of a file.
    body: Body,
}

enum Body {
    Bytes(Vec<u8>),
    File { path: PathBuf, start: u64, len: u64 },
}

fn text_response(status: &'static str, text: &str) -> Response {
    Response {
        status,
        headers: vec![("Content-Type".into(), "text/plain; charset=utf-8".into())],
        body: Body::Bytes(text.as_bytes().to_vec()),
    }
}

/// Builds the answer to `method target` given the request headers.
fn answer(root: &Path, method: &str, target: &str, headers: &HashMap<String, String>) -> Response {
    if method != "GET" && method != "HEAD" {
        return text_response("405 Method Not Allowed", "method not allowed");
    }
    let Some(path) = resolve(root, target) else {
        return text_response("400 Bad Request", "bad path");
    };
    if !path.is_file() {
        return text_response("404 Not Found", "not found");
    }
    let mut served = path.clone();
    let mut encoding: Option<&str> = None;
    let accept = headers.get("accept-encoding");
    for (coding, ext) in [("br", "br"), ("gzip", "gz")] {
        let sibling = PathBuf::from(format!("{}.{ext}", path.display()));
        if wants(accept, coding) && sibling.is_file() {
            served = sibling;
            encoding = Some(coding);
            break;
        }
    }
    let Ok(meta) = std::fs::metadata(&served) else {
        return text_response("404 Not Found", "not found");
    };
    let len = meta.len();
    let mut out_headers = vec![
        ("Content-Type".to_owned(), mime_type(&path).to_owned()),
        ("Accept-Ranges".to_owned(), "bytes".to_owned()),
        ("Cache-Control".to_owned(), "no-cache".to_owned()),
        ("Vary".to_owned(), "Accept-Encoding".to_owned()),
    ];
    if let Some(coding) = encoding {
        out_headers.push(("Content-Encoding".to_owned(), coding.to_owned()));
    }
    // A compressed representation is always sent whole.
    let range = if encoding.is_some() {
        RangeRequest::Whole
    } else {
        parse_range(headers.get("range"), len)
    };
    match range {
        RangeRequest::Whole => Response {
            status: "200 OK",
            headers: out_headers,
            body: Body::File {
                path: served,
                start: 0,
                len,
            },
        },
        RangeRequest::Part(start, end) => {
            out_headers.push(("Content-Range".to_owned(), format!("bytes {start}-{end}/{len}")));
            Response {
                status: "206 Partial Content",
                headers: out_headers,
                body: Body::File {
                    path: served,
                    start,
                    len: end - start + 1,
                },
            }
        }
        RangeRequest::Unsatisfiable => Response {
            status: "416 Range Not Satisfiable",
            headers: vec![("Content-Range".to_owned(), format!("bytes */{len}"))],
            body: Body::Bytes(Vec::new()),
        },
    }
}

fn body_len(body: &Body) -> u64 {
    match body {
        Body::Bytes(b) => b.len() as u64,
        Body::File { len, .. } => *len,
    }
}

fn handle(mut stream: TcpStream, root: &Path, log: Option<&(dyn Fn(String) + Send + Sync)>) -> io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    let mut headers = HashMap::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 || line == "\r\n" || line == "\n" {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_owned());
        }
    }
    let mut parts = request_line.split_whitespace();
    let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or("/"));
    let head_only = method == "HEAD";
    let response = answer(root, method, target, &headers);

    if let Some(log) = log {
        let range = headers.get("range").map_or(String::new(), |r| format!(" {r}"));
        log(format!(
            "{method} {target} {}{range}",
            response.status.split(' ').next().unwrap_or("")
        ));
    }

    let mut head = format!("HTTP/1.1 {}\r\n", response.status);
    for (k, v) in &response.headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str(&format!("Content-Length: {}\r\nConnection: close\r\n\r\n", body_len(&response.body)));
    stream.write_all(head.as_bytes())?;
    if head_only {
        return Ok(());
    }
    match response.body {
        Body::Bytes(bytes) => stream.write_all(&bytes)?,
        Body::File { path, start, len } => {
            let mut file = File::open(path)?;
            file.seek(SeekFrom::Start(start))?;
            io::copy(&mut file.take(len), &mut stream)?;
        }
    }
    stream.flush()
}

/// Opens a file or a folder with the system (the file manager for a folder).
pub fn open_path(path: &Path) -> io::Result<()> {
    use std::process::{Command, Stdio};
    let mut command = if cfg!(target_os = "windows") {
        Command::new("explorer")
    } else if cfg!(target_os = "macos") {
        Command::new("open")
    } else {
        Command::new("xdg-open")
    };
    command
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
}

/// Opens `url` in the default browser.
pub fn open_in_browser(url: &str) -> io::Result<()> {
    use std::process::{Command, Stdio};
    let mut command = if cfg!(target_os = "windows") {
        let mut c = Command::new("cmd");
        // The empty string is the window title `start` wants before a URL.
        c.args(["/C", "start", "", url]);
        c
    } else if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg(url);
        c
    } else {
        let mut c = Command::new("xdg-open");
        c.arg(url);
        c
    };
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    fn get(port: u16, path: &str, extra: &str) -> (String, Vec<u8>) {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(stream, "GET {path} HTTP/1.1\r\nHost: x\r\n{extra}\r\n").unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).unwrap();
        let split = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        (
            String::from_utf8_lossy(&raw[..split]).into_owned(),
            raw[split + 4..].to_vec(),
        )
    }

    #[test]
    fn paths_cannot_escape_the_root() {
        let root = Path::new("root");
        assert_eq!(resolve(root, "/a/b.js?v=1"), Some(root.join("a").join("b.js")));
        assert_eq!(resolve(root, "/%2e%2e/x"), None);
        assert_eq!(resolve(root, "/../x"), None);
        assert_eq!(resolve(root, "/a\\b"), None);
        assert_eq!(resolve(root, "/c:/x"), None);
    }

    #[test]
    fn range_headers() {
        let h = |s: &str| Some(s.to_owned());
        assert_eq!(parse_range(None, 100), RangeRequest::Whole);
        assert_eq!(parse_range(h("bytes=0-9").as_ref(), 100), RangeRequest::Part(0, 9));
        assert_eq!(parse_range(h("bytes=90-").as_ref(), 100), RangeRequest::Part(90, 99));
        assert_eq!(parse_range(h("bytes=-10").as_ref(), 100), RangeRequest::Part(90, 99));
        assert_eq!(parse_range(h("bytes=0-1000").as_ref(), 100), RangeRequest::Part(0, 99));
        assert_eq!(parse_range(h("bytes=200-300").as_ref(), 100), RangeRequest::Unsatisfiable);
        assert_eq!(parse_range(h("bytes=5-2").as_ref(), 100), RangeRequest::Unsatisfiable);
        assert_eq!(parse_range(h("items=1-2").as_ref(), 100), RangeRequest::Whole);
    }

    #[test]
    fn serves_files_ranges_and_precompressed_siblings() {
        let dir = TempDir::new("serve");
        std::fs::write(dir.path().join("index.html"), "<html>").unwrap();
        std::fs::write(dir.path().join("game.pak"), (0u8..=255).collect::<Vec<_>>()).unwrap();
        std::fs::write(dir.path().join("a.js"), "console.log(1)").unwrap();
        std::fs::write(dir.path().join("a.js.br"), "BR").unwrap();
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = seen.clone();
        let server = Server::start(
            dir.path().to_owned(),
            None,
            Some(Arc::new(move |line| sink.lock().unwrap().push(line))),
        )
        .unwrap();
        let port = server.port();

        let (head, body) = get(port, "/", "");
        assert!(head.starts_with("HTTP/1.1 200"), "{head}");
        assert_eq!(body, b"<html>");

        let (head, body) = get(port, "/game.pak?v=1", "Range: bytes=10-13\r\n");
        assert!(head.starts_with("HTTP/1.1 206"), "{head}");
        assert!(head.contains("Content-Range: bytes 10-13/256"), "{head}");
        assert_eq!(body, [10, 11, 12, 13]);

        let (head, _) = get(port, "/game.pak", "Range: bytes=500-600\r\n");
        assert!(head.starts_with("HTTP/1.1 416"), "{head}");

        let (head, body) = get(port, "/a.js", "Accept-Encoding: gzip, br\r\n");
        assert!(head.contains("Content-Encoding: br"), "{head}");
        assert_eq!(body, b"BR");
        let (head, body) = get(port, "/a.js", "");
        assert!(!head.contains("Content-Encoding"), "{head}");
        assert_eq!(body, b"console.log(1)");

        assert!(get(port, "/nope", "").0.starts_with("HTTP/1.1 404"));
        assert!(get(port, "/../x", "").0.starts_with("HTTP/1.1 400"));
        assert!(seen.lock().unwrap().iter().any(|l| l.contains("/game.pak?v=1 206 bytes=10-13")));
        server.stop();
    }
}
