//! A minimal blocking HTTP client, without new dependencies (moved here from
//! `orbit_ai_studio::http`, which re-exports it):
//!
//! * `http://` (local Ollama, LM Studio, ComfyUI, test servers): HTTP/1.1
//!   over `std::net::TcpStream` (`Content-Length`, chunked or until close),
//!   redirects followed;
//! * `https://`: the system `curl` executable (shipped with Windows 10+,
//!   macOS and most Linux), so the editor carries no TLS stack; `curl` is
//!   limited to http(s), also on redirects;
//! * `file://` URLs (local catalogs, tests): read from disk.
//!
//! [`Request::send`] returns the whole body (bounded); [`Request::download`]
//! streams it to a file with progress and cancellation.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

/// Largest answer [`Request::send`] reads (images can be big).
const MAX_BODY: usize = 256 * 1024 * 1024;
/// Redirects followed (plain http).
const MAX_REDIRECTS: usize = 5;

#[derive(Debug)]
pub struct Response {
    pub status: u16,
    /// Lowercase names, in order (the final response's, after redirects).
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    /// The first header `name` (lowercase).
    pub fn header(&self, name: &str) -> Option<&str> {
        header_in(&self.headers, name)
    }
}

fn header_in<'h>(headers: &'h [(String, String)], name: &str) -> Option<&'h str> {
    headers
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
}

/// What a finished [`Request::download`] got (the body is in the file).
#[derive(Debug)]
pub struct Downloaded {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    /// Bytes written.
    pub size: u64,
}

/// Why a download stopped.
#[derive(Debug, PartialEq, Eq)]
pub enum DownloadError {
    /// `progress` asked to stop.
    Cancelled,
    /// More bytes than the limit given.
    TooLarge,
    Failed(String),
}

impl std::fmt::Display for DownloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => write!(f, "cancelled"),
            Self::TooLarge => write!(f, "the file is larger than announced"),
            Self::Failed(e) => write!(f, "{e}"),
        }
    }
}

impl From<String> for DownloadError {
    fn from(e: String) -> Self {
        Self::Failed(e)
    }
}

/// Called while downloading with (bytes received, total when known); returns
/// `false` to cancel.
pub type ProgressFn<'p> = &'p dyn Fn(u64, Option<u64>) -> bool;

#[derive(Clone, Debug)]
pub struct Request<'a> {
    pub method: &'a str,
    pub url: &'a str,
    pub headers: Vec<(&'a str, String)>,
    pub body: Option<Vec<u8>>,
    pub timeout: Duration,
    pub user_agent: &'a str,
}

impl<'a> Request<'a> {
    pub fn get(url: &'a str, timeout: Duration) -> Self {
        Self {
            method: "GET",
            url,
            headers: Vec::new(),
            body: None,
            timeout,
            user_agent: "orbit-engine",
        }
    }

    pub fn post_json(url: &'a str, body: &serde_json::Value, timeout: Duration) -> Self {
        Self {
            method: "POST",
            url,
            headers: vec![("Content-Type", "application/json".to_owned())],
            body: Some(body.to_string().into_bytes()),
            timeout,
            user_agent: "orbit-engine",
        }
    }

    pub fn bearer(mut self, key: &str) -> Self {
        if !key.is_empty() {
            self.headers
                .push(("Authorization", format!("Bearer {key}")));
        }
        self
    }

    pub fn header(mut self, name: &'a str, value: impl Into<String>) -> Self {
        self.headers.push((name, value.into()));
        self
    }

    pub fn user_agent(mut self, agent: &'a str) -> Self {
        self.user_agent = agent;
        self
    }

    pub fn send(&self) -> Result<Response, String> {
        if let Some(path) = file_url_path(self.url) {
            return read_file_url(&path);
        }
        if self.url.starts_with("http://") {
            let mut url = self.url.to_owned();
            for _ in 0..=MAX_REDIRECTS {
                let stream = self.connect(&url)?;
                let mut reader = BufReader::new(stream);
                let (status, _, headers) = read_head(&mut reader)?;
                if let Some(next) = redirect_target(status, &headers, &url) {
                    url = next;
                    continue;
                }
                let body = read_body(&mut reader, &headers, MAX_BODY as u64)?;
                return Ok(Response {
                    status,
                    headers,
                    body,
                });
            }
            Err(format!("too many redirects from {}", self.url))
        } else if self.url.starts_with("https://") {
            self.send_curl()
        } else {
            Err(format!(
                "unsupported URL `{}` (http://, https:// or file://)",
                self.url
            ))
        }
    }

    /// Streams the answer to `dest` (created or replaced). The status is
    /// returned whatever it is: a non-2xx body is in the file too. `limit`:
    /// the download fails beyond that many bytes.
    pub fn download(
        &self,
        dest: &Path,
        limit: u64,
        progress: ProgressFn<'_>,
    ) -> Result<Downloaded, DownloadError> {
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        if let Some(path) = file_url_path(self.url) {
            return copy_file(&path, dest, limit, progress);
        }
        if self.url.starts_with("http://") {
            let mut url = self.url.to_owned();
            for _ in 0..=MAX_REDIRECTS {
                let stream = self.connect(&url)?;
                let mut reader = BufReader::new(stream);
                let (status, _, headers) = read_head(&mut reader)?;
                if let Some(next) = redirect_target(status, &headers, &url) {
                    url = next;
                    continue;
                }
                let total =
                    header_in(&headers, "content-length").and_then(|v| v.parse::<u64>().ok());
                let mut file = std::fs::File::create(dest).map_err(|e| e.to_string())?;
                let mut size = 0u64;
                let mut cancelled = false;
                let mut too_large = false;
                let mut sink = |chunk: &[u8]| {
                    size += chunk.len() as u64;
                    if size > limit {
                        too_large = true;
                        return false;
                    }
                    if file.write_all(chunk).is_err() || !progress(size, total) {
                        cancelled = true;
                        return false;
                    }
                    true
                };
                let result = stream_body(&mut reader, &headers, &mut sink);
                if too_large {
                    return Err(DownloadError::TooLarge);
                }
                if cancelled {
                    return Err(DownloadError::Cancelled);
                }
                result?;
                file.flush().map_err(|e| e.to_string())?;
                return Ok(Downloaded {
                    status,
                    headers,
                    size,
                });
            }
            Err(DownloadError::Failed(format!(
                "too many redirects from {}",
                self.url
            )))
        } else if self.url.starts_with("https://") {
            self.download_curl(dest, limit, progress)
        } else {
            Err(DownloadError::Failed(format!(
                "unsupported URL `{}`",
                self.url
            )))
        }
    }

    fn connect(&self, url: &str) -> Result<TcpStream, String> {
        let rest = url
            .strip_prefix("http://")
            .ok_or_else(|| format!("not an http URL: {url}"))?;
        let (authority, path) = match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, "/"),
        };
        let host_port = if authority.contains(':') && !authority.ends_with(']') {
            authority.to_owned()
        } else {
            format!("{authority}:80")
        };
        // `localhost` resolves to ::1 first on Windows while Ollama / ComfyUI
        // listen on 127.0.0.1 only: try every address, IPv4 first.
        let mut addrs: Vec<_> = host_port
            .to_socket_addrs()
            .map_err(|e| format!("{authority}: {e}"))?
            .collect();
        addrs.sort_by_key(|a| !a.is_ipv4());
        let mut last = format!("{authority}: no address");
        let mut stream = None;
        for addr in addrs {
            match TcpStream::connect_timeout(&addr, self.timeout.min(Duration::from_secs(15))) {
                Ok(s) => {
                    stream = Some(s);
                    break;
                }
                Err(e) => last = format!("cannot connect to {authority}: {e}"),
            }
        }
        let mut stream = stream.ok_or(last)?;
        stream.set_read_timeout(Some(self.timeout)).ok();
        stream.set_write_timeout(Some(self.timeout)).ok();
        let mut head = format!(
            "{} {path} HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\nAccept: */*\r\nUser-Agent: {}\r\n",
            self.method, self.user_agent
        );
        for (k, v) in &self.headers {
            head.push_str(&format!("{k}: {v}\r\n"));
        }
        if let Some(body) = &self.body {
            head.push_str(&format!("Content-Length: {}\r\n", body.len()));
        }
        head.push_str("\r\n");
        stream
            .write_all(head.as_bytes())
            .map_err(|e| e.to_string())?;
        if let Some(body) = &self.body {
            stream.write_all(body).map_err(|e| e.to_string())?;
        }
        stream.flush().map_err(|e| e.to_string())?;
        Ok(stream)
    }

    fn curl(&self) -> Command {
        let mut cmd = Command::new("curl");
        cmd.arg("-sS")
            .arg("-L")
            .args(["--max-redirs", &MAX_REDIRECTS.to_string()])
            // Never let a redirect reach file:// or another protocol.
            .args(["--proto", "=http,https", "--proto-redir", "=http,https"])
            .arg("--max-time")
            .arg(self.timeout.as_secs().max(1).to_string())
            .args(["-A", self.user_agent])
            .arg("-X")
            .arg(self.method);
        for (k, v) in &self.headers {
            cmd.arg("-H").arg(format!("{k}: {v}"));
        }
        cmd
    }

    fn send_curl(&self) -> Result<Response, String> {
        let mut cmd = self.curl();
        cmd.arg("-i");
        if self.body.is_some() {
            cmd.arg("--data-binary").arg("@-");
        }
        cmd.arg(self.url)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("https needs the `curl` program ({e})"))?;
        if let (Some(body), Some(mut stdin)) = (&self.body, child.stdin.take()) {
            stdin.write_all(body).map_err(|e| e.to_string())?;
        }
        let out = child.wait_with_output().map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(format!(
                "curl: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        let (status, headers, consumed) = last_head(&out.stdout)?;
        Ok(Response {
            status,
            headers,
            body: out.stdout[consumed..].to_vec(),
        })
    }

    fn download_curl(
        &self,
        dest: &Path,
        limit: u64,
        progress: ProgressFn<'_>,
    ) -> Result<Downloaded, DownloadError> {
        let head_file = PathBuf::from(format!("{}.headers", dest.display()));
        let mut cmd = self.curl();
        cmd.arg("-D")
            .arg(&head_file)
            .arg("-o")
            .arg(dest)
            .args(["--max-filesize", &limit.to_string()])
            .arg(self.url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let mut child = cmd
            .spawn()
            .map_err(|e| DownloadError::Failed(format!("https needs the `curl` program ({e})")))?;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => {}
                Err(e) => return Err(DownloadError::Failed(e.to_string())),
            }
            let size = std::fs::metadata(dest).map_or(0, |m| m.len());
            if size > limit {
                let _ = child.kill();
                let _ = child.wait();
                let _ = std::fs::remove_file(&head_file);
                return Err(DownloadError::TooLarge);
            }
            if !progress(size, None) {
                let _ = child.kill();
                let _ = child.wait();
                let _ = std::fs::remove_file(&head_file);
                return Err(DownloadError::Cancelled);
            }
            std::thread::sleep(Duration::from_millis(100));
        };
        let heads = std::fs::read(&head_file).unwrap_or_default();
        let _ = std::fs::remove_file(&head_file);
        if !status.success() {
            let mut err = String::new();
            if let Some(mut e) = child.stderr.take() {
                let _ = e.read_to_string(&mut err);
            }
            if status.code() == Some(63) {
                return Err(DownloadError::TooLarge);
            }
            return Err(DownloadError::Failed(format!("curl: {}", err.trim())));
        }
        let (code, headers, _) = last_head(&heads)?;
        let size = std::fs::metadata(dest).map_or(0, |m| m.len());
        Ok(Downloaded {
            status: code,
            headers,
            size,
        })
    }
}

/// The local path of a `file://` URL (`file:///C:/x`, `file:///home/x`,
/// `file://localhost/x`); percent escapes are decoded.
pub fn file_url_path(url: &str) -> Option<PathBuf> {
    let rest = url.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    let decoded = percent_decode(rest);
    // `/C:/dir` on Windows.
    let bytes = decoded.as_bytes();
    let path = if bytes.len() > 2 && bytes[0] == b'/' && bytes[2] == b':' {
        &decoded[1..]
    } else {
        &decoded[..]
    };
    Some(PathBuf::from(path))
}

/// A `file://` URL for a local path.
pub fn file_url(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    let text = text.strip_prefix("//?/").unwrap_or(&text).to_owned();
    let mut out = String::from("file://");
    if !text.starts_with('/') {
        out.push('/');
    }
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~/:".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Some(v) = text
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn read_file_url(path: &Path) -> Result<Response, String> {
    match std::fs::read(path) {
        Ok(body) => Ok(Response {
            status: 200,
            headers: Vec::new(),
            body,
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Response {
            status: 404,
            headers: Vec::new(),
            body: Vec::new(),
        }),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

fn copy_file(
    from: &Path,
    dest: &Path,
    limit: u64,
    progress: ProgressFn<'_>,
) -> Result<Downloaded, DownloadError> {
    let mut input = match std::fs::File::open(from) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Downloaded {
                status: 404,
                headers: Vec::new(),
                size: 0,
            });
        }
        Err(e) => return Err(DownloadError::Failed(format!("{}: {e}", from.display()))),
    };
    let total = input.metadata().ok().map(|m| m.len());
    let mut out = std::fs::File::create(dest).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; 256 * 1024];
    let mut size = 0u64;
    loop {
        let n = input.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        size += n as u64;
        if size > limit {
            return Err(DownloadError::TooLarge);
        }
        out.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        if !progress(size, total) {
            return Err(DownloadError::Cancelled);
        }
    }
    Ok(Downloaded {
        status: 200,
        headers: Vec::new(),
        size,
    })
}

/// Where a 3xx answer points (`Location`, relative or absolute).
fn redirect_target(status: u16, headers: &[(String, String)], url: &str) -> Option<String> {
    if !matches!(status, 301 | 302 | 303 | 307 | 308) {
        return None;
    }
    let location = header_in(headers, "location")?;
    Some(resolve_url(url, location))
}

/// `reference` resolved against `base` (absolute URL, `/path` or relative
/// path). Enough for catalogs and redirects; not a full RFC 3986 resolver.
pub fn resolve_url(base: &str, reference: &str) -> String {
    if reference.contains("://") {
        return reference.to_owned();
    }
    let scheme_end = base.find("://").map_or(0, |i| i + 3);
    let path_start = base[scheme_end..]
        .find('/')
        .map_or(base.len(), |i| scheme_end + i);
    if reference.starts_with('/') {
        return format!("{}{reference}", &base[..path_start]);
    }
    let base_path = base.split(['?', '#']).next().unwrap_or(base);
    let dir_end = base_path
        .rfind('/')
        .filter(|&i| i >= path_start)
        .map_or(base_path.len(), |i| i + 1);
    let mut out = base_path[..dir_end].to_owned();
    if !out.ends_with('/') {
        out.push('/');
    }
    let mut rest = reference;
    loop {
        if let Some(r) = rest.strip_prefix("./") {
            rest = r;
        } else if let Some(r) = rest.strip_prefix("../") {
            rest = r;
            let trimmed = out.trim_end_matches('/');
            if let Some(i) = trimmed.rfind('/')
                && i + 1 > path_start
            {
                out.truncate(i + 1);
            }
        } else {
            break;
        }
    }
    out.push_str(rest);
    out
}

/// Reads the status line and headers; returns the status, the bytes
/// consumed and the headers (lowercase names).
type Head = (u16, usize, Vec<(String, String)>);

fn read_head<R: BufRead>(mut r: R) -> Result<Head, String> {
    let mut consumed = 0;
    let mut line = String::new();
    consumed += r.read_line(&mut line).map_err(|e| e.to_string())?;
    let status = line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or_else(|| format!("bad HTTP status line `{}`", line.trim()))?;
    let mut headers = Vec::new();
    loop {
        line.clear();
        let n = r.read_line(&mut line).map_err(|e| e.to_string())?;
        consumed += n;
        if n == 0 || line.trim().is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            headers.push((k.trim().to_ascii_lowercase(), v.trim().to_owned()));
        }
    }
    Ok((status, consumed, headers))
}

/// The last header block of curl's output (`-i` or `-D`): 1xx answers and
/// the redirects it followed come first.
/// Status, headers and the bytes consumed.
type LastHead = (u16, Vec<(String, String)>, usize);

fn last_head(mut data: &[u8]) -> Result<LastHead, String> {
    let start = data.len();
    loop {
        let (status, consumed, headers) = read_head(BufReader::new(data))?;
        data = &data[consumed..];
        let another = data.starts_with(b"HTTP/");
        if another && ((100..200).contains(&status) || (300..400).contains(&status)) {
            continue;
        }
        return Ok((status, headers, start - data.len()));
    }
}

/// Feeds the body to `sink` in pieces until it returns `false`.
fn stream_body<R: BufRead>(
    r: &mut R,
    headers: &[(String, String)],
    sink: &mut dyn FnMut(&[u8]) -> bool,
) -> Result<(), String> {
    let mut buf = vec![0u8; 64 * 1024];
    if header_in(headers, "transfer-encoding").is_some_and(|v| v.eq_ignore_ascii_case("chunked")) {
        loop {
            let mut size = String::new();
            r.read_line(&mut size).map_err(|e| e.to_string())?;
            let mut left = usize::from_str_radix(size.trim().split(';').next().unwrap_or("0"), 16)
                .map_err(|_| "bad chunk size".to_owned())?;
            if left == 0 {
                return Ok(());
            }
            while left > 0 {
                let n = left.min(buf.len());
                r.read_exact(&mut buf[..n]).map_err(|e| e.to_string())?;
                if !sink(&buf[..n]) {
                    return Ok(());
                }
                left -= n;
            }
            let mut crlf = [0u8; 2];
            r.read_exact(&mut crlf).map_err(|e| e.to_string())?;
        }
    } else if let Some(len) =
        header_in(headers, "content-length").and_then(|v| v.parse::<u64>().ok())
    {
        let mut left = len;
        while left > 0 {
            let n = (left as usize).min(buf.len());
            r.read_exact(&mut buf[..n]).map_err(|e| e.to_string())?;
            if !sink(&buf[..n]) {
                return Ok(());
            }
            left -= n as u64;
        }
        Ok(())
    } else {
        loop {
            let n = r.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 || !sink(&buf[..n]) {
                return Ok(());
            }
        }
    }
}

fn read_body<R: BufRead>(
    r: &mut R,
    headers: &[(String, String)],
    limit: u64,
) -> Result<Vec<u8>, String> {
    let mut body = Vec::new();
    let mut too_large = false;
    stream_body(r, headers, &mut |chunk| {
        if body.len() as u64 + chunk.len() as u64 > limit {
            too_large = true;
            return false;
        }
        body.extend_from_slice(chunk);
        true
    })?;
    if too_large {
        return Err("answer too large".to_owned());
    }
    Ok(body)
}

/// Percent-encodes a query value.
pub fn encode_query(v: &str) -> String {
    let mut out = String::new();
    for b in v.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub mod testing;

#[cfg(test)]
mod tests;
