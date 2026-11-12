//! A tiny HTTP/1.1 server on `127.0.0.1` for tests (no network): fixed
//! answers by path, every request recorded. Used by this crate's and the
//! plugin store's tests.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

/// One canned answer.
#[derive(Clone, Debug)]
pub struct Answer {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// Send the body chunked instead of with `Content-Length`.
    pub chunked: bool,
}

impl Answer {
    pub fn ok(body: impl Into<Vec<u8>>) -> Self {
        Self {
            status: 200,
            headers: Vec::new(),
            body: body.into(),
            chunked: false,
        }
    }

    pub fn status(status: u16) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: Vec::new(),
            chunked: false,
        }
    }

    pub fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }

    pub fn chunked(mut self) -> Self {
        self.chunked = true;
        self
    }
}

/// A request the server got: path and headers (lowercase names).
#[derive(Clone, Debug)]
pub struct Seen {
    pub path: String,
    pub headers: Vec<(String, String)>,
}

impl Seen {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

type Handler = dyn Fn(&Seen) -> Option<Answer> + Send + Sync;

pub struct Server {
    pub base: String,
    answers: Arc<Mutex<BTreeMap<String, Answer>>>,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl Server {
    /// Starts the server (a background thread that lives as long as the
    /// test process).
    pub fn start() -> Self {
        Self::start_with(None)
    }

    /// `handler` answers first (for conditional requests); `None` falls back
    /// to the canned answers.
    pub fn start_with(handler: Option<Box<Handler>>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind 127.0.0.1");
        let base = format!("http://{}", listener.local_addr().expect("address"));
        let answers: Arc<Mutex<BTreeMap<String, Answer>>> = Arc::default();
        let seen: Arc<Mutex<Vec<Seen>>> = Arc::default();
        let (a, s) = (answers.clone(), seen.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut reader = BufReader::new(match stream.try_clone() {
                    Ok(s) => s,
                    Err(_) => continue,
                });
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() {
                    continue;
                }
                let path = line.split_whitespace().nth(1).unwrap_or("/").to_owned();
                let mut headers = Vec::new();
                loop {
                    let mut h = String::new();
                    if reader.read_line(&mut h).unwrap_or(0) == 0 || h.trim().is_empty() {
                        break;
                    }
                    if let Some((k, v)) = h.split_once(':') {
                        headers.push((k.trim().to_ascii_lowercase(), v.trim().to_owned()));
                    }
                }
                let request = Seen {
                    path: path.clone(),
                    headers,
                };
                s.lock().unwrap().push(request.clone());
                let answer = handler
                    .as_ref()
                    .and_then(|h| h(&request))
                    .or_else(|| a.lock().unwrap().get(&path).cloned())
                    .unwrap_or_else(|| Answer::status(404));
                let mut head = format!("HTTP/1.1 {} X\r\nConnection: close\r\n", answer.status);
                for (k, v) in &answer.headers {
                    head.push_str(&format!("{k}: {v}\r\n"));
                }
                if answer.chunked {
                    head.push_str("Transfer-Encoding: chunked\r\n\r\n");
                    let _ = stream.write_all(head.as_bytes());
                    for chunk in answer.body.chunks(1000) {
                        let _ = stream.write_all(format!("{:x}\r\n", chunk.len()).as_bytes());
                        let _ = stream.write_all(chunk);
                        let _ = stream.write_all(b"\r\n");
                    }
                    let _ = stream.write_all(b"0\r\n\r\n");
                } else {
                    head.push_str(&format!("Content-Length: {}\r\n\r\n", answer.body.len()));
                    let _ = stream.write_all(head.as_bytes());
                    let _ = stream.write_all(&answer.body);
                }
                let _ = stream.flush();
            }
        });
        Self {
            base,
            answers,
            seen,
        }
    }

    /// Serves `answer` at `path` (from now on).
    pub fn route(&self, path: &str, answer: Answer) {
        self.answers.lock().unwrap().insert(path.to_owned(), answer);
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    /// The requests received so far.
    pub fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}
