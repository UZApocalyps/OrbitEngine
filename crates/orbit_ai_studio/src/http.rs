//! A minimal blocking HTTP client, without new dependencies:
//!
//! * `http://` (local Ollama, LM Studio, ComfyUI, A1111... the common case):
//!   HTTP/1.1 over `std::net::TcpStream` (`Content-Length` or chunked
//!   answers);
//! * `https://` (OpenRouter, OpenAI...): the system `curl` executable
//!   (shipped with Windows 10+, macOS and most Linux), so the editor carries
//!   no TLS stack.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::process::{Command, Stdio};
use std::time::Duration;

/// Largest answer read (images can be big).
const MAX_BODY: usize = 256 * 1024 * 1024;

#[derive(Debug)]
pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
}

impl Response {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

#[derive(Clone, Debug)]
pub struct Request<'a> {
    pub method: &'a str,
    pub url: &'a str,
    pub headers: Vec<(&'a str, String)>,
    pub body: Option<Vec<u8>>,
    pub timeout: Duration,
}

impl<'a> Request<'a> {
    pub fn get(url: &'a str, timeout: Duration) -> Self {
        Self { method: "GET", url, headers: Vec::new(), body: None, timeout }
    }

    pub fn post_json(url: &'a str, body: &serde_json::Value, timeout: Duration) -> Self {
        Self {
            method: "POST",
            url,
            headers: vec![("Content-Type", "application/json".to_owned())],
            body: Some(body.to_string().into_bytes()),
            timeout,
        }
    }

    pub fn bearer(mut self, key: &str) -> Self {
        if !key.is_empty() {
            self.headers.push(("Authorization", format!("Bearer {key}")));
        }
        self
    }

    pub fn send(&self) -> Result<Response, String> {
        if let Some(rest) = self.url.strip_prefix("http://") {
            self.send_plain(rest)
        } else if self.url.starts_with("https://") {
            self.send_curl()
        } else {
            Err(format!("unsupported URL `{}` (http:// or https://)", self.url))
        }
    }

    fn send_plain(&self, rest: &str) -> Result<Response, String> {
        let (authority, path) = match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, "/"),
        };
        let host_port = if authority.contains(':') && !authority.ends_with(']') {
            authority.to_owned()
        } else {
            format!("{authority}:80")
        };
        let addr = host_port
            .to_socket_addrs()
            .map_err(|e| format!("{authority}: {e}"))?
            .next()
            .ok_or_else(|| format!("{authority}: no address"))?;
        let mut stream = TcpStream::connect_timeout(&addr, self.timeout.min(Duration::from_secs(15)))
            .map_err(|e| format!("cannot connect to {authority}: {e}"))?;
        stream.set_read_timeout(Some(self.timeout)).ok();
        stream.set_write_timeout(Some(self.timeout)).ok();
        let mut head = format!(
            "{} {path} HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\nAccept: */*\r\nUser-Agent: orbit-ai-studio\r\n",
            self.method
        );
        for (k, v) in &self.headers {
            head.push_str(&format!("{k}: {v}\r\n"));
        }
        if let Some(body) = &self.body {
            head.push_str(&format!("Content-Length: {}\r\n", body.len()));
        }
        head.push_str("\r\n");
        stream.write_all(head.as_bytes()).map_err(|e| e.to_string())?;
        if let Some(body) = &self.body {
            stream.write_all(body).map_err(|e| e.to_string())?;
        }
        stream.flush().map_err(|e| e.to_string())?;
        read_response(BufReader::new(stream))
    }

    fn send_curl(&self) -> Result<Response, String> {
        let mut cmd = Command::new("curl");
        cmd.arg("-sS")
            .arg("-i")
            .arg("--max-time")
            .arg(self.timeout.as_secs().max(1).to_string())
            .arg("-X")
            .arg(self.method);
        for (k, v) in &self.headers {
            cmd.arg("-H").arg(format!("{k}: {v}"));
        }
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
            return Err(format!("curl: {}", String::from_utf8_lossy(&out.stderr).trim()));
        }
        // `-i` puts the header blocks first (several with 1xx / proxies).
        let mut data: &[u8] = &out.stdout;
        loop {
            let reader = BufReader::new(data);
            let (status, consumed, _) = read_head(reader)?;
            data = &data[consumed..];
            if !(100..200).contains(&status) {
                return Ok(Response { status, body: data.to_vec() });
            }
        }
    }
}

/// Reads the status line and headers; returns the status, the bytes
/// consumed and the headers (lowercase names).
fn read_head<R: BufRead>(mut r: R) -> Result<(u16, usize, Vec<(String, String)>), String> {
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

fn read_response<R: BufRead>(mut r: R) -> Result<Response, String> {
    let (status, _, headers) = read_head(&mut r)?;
    let header = |name: &str| headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str());
    let mut body = Vec::new();
    if header("transfer-encoding").is_some_and(|v| v.eq_ignore_ascii_case("chunked")) {
        loop {
            let mut size = String::new();
            r.read_line(&mut size).map_err(|e| e.to_string())?;
            let size = usize::from_str_radix(size.trim().split(';').next().unwrap_or("0"), 16)
                .map_err(|_| "bad chunk size".to_owned())?;
            if size == 0 {
                break;
            }
            if body.len() + size > MAX_BODY {
                return Err("answer too large".to_owned());
            }
            let start = body.len();
            body.resize(start + size, 0);
            r.read_exact(&mut body[start..]).map_err(|e| e.to_string())?;
            let mut crlf = [0u8; 2];
            r.read_exact(&mut crlf).map_err(|e| e.to_string())?;
        }
    } else if let Some(len) = header("content-length").and_then(|v| v.parse::<usize>().ok()) {
        if len > MAX_BODY {
            return Err("answer too large".to_owned());
        }
        body.resize(len, 0);
        r.read_exact(&mut body).map_err(|e| e.to_string())?;
    } else {
        r.take(MAX_BODY as u64).read_to_end(&mut body).map_err(|e| e.to_string())?;
    }
    Ok(Response { status, body })
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
