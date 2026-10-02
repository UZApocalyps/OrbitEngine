//! Live mode (M4): the MCP server that runs **inside the editor**.
//!
//! ```text
//!  client ──HTTP──▶ [server thread: tokio, auth, rmcp]
//!                        │ OrbitMcpServer worker thread (one call at a time)
//!                        ▼
//!                  LiveHost (ProjectHost) ──▶ CommandQueue ──▶ editor thread
//!                        ▲                                      │ runs the tool
//!                        └────────────── reply (channel) ◀──────┘ on the live Project
//! ```
//!
//! * The server only ever listens on **127.0.0.1**; every request needs the
//!   access token (`Authorization: Bearer <token>`); `Host` is checked by
//!   `rmcp` against DNS rebinding.
//! * It never touches a project: it posts [`LiveJob`]s to the shared
//!   [`CommandQueue`]. The editor drains the queue on its own thread (the
//!   `Project`, the registry and the GPU are not `Send`), runs the tool with
//!   [`crate::host::ToolRunner`] (one undo entry per call) and answers. A
//!   [`CommandQueue::set_waker`] callback wakes the editor's event loop.

use std::collections::VecDeque;
use std::convert::Infallible;
use std::net::{Ipv4Addr, SocketAddr, TcpListener};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use bytes::Bytes;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::header::{AUTHORIZATION, CONTENT_TYPE, WWW_AUTHENTICATE};
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use serde_json::Value as Json;
use tokio_util::sync::CancellationToken;

use crate::error::{ErrorKind, ToolError};
use crate::host::{ExportRequest, ProjectHost, ResourceInfo, ToolInfo, ToolOutput};
use crate::live_tools::{self, LiveCall};
use crate::server::{LIVE_INSTRUCTIONS, OrbitMcpServer};

/// How long a request waits for the editor to answer (a sleeping or blocked
/// editor must not hang the client for ever).
const REPLY_TIMEOUT: Duration = Duration::from_secs(60);
/// Builds and exports take longer.
const EXPORT_TIMEOUT: Duration = Duration::from_secs(30 * 60);

// ------------------------------------------------------------ the queue

/// What the server asks of the editor.
#[derive(Debug)]
pub enum LiveRequest {
    ListTools,
    CallTool { name: String, args: Json },
    ListResources,
    ReadResource { uri: String },
    Export(ExportRequest),
}

/// The editor's answer.
#[derive(Debug)]
pub enum LiveReply {
    Tools(Vec<ToolInfo>),
    Tool(Result<ToolOutput, ToolError>),
    Resources(Vec<ResourceInfo>),
    Resource(Result<(&'static str, String), ToolError>),
    Export(Result<String, ToolError>),
}

/// A request and the way back.
#[derive(Debug)]
pub struct LiveJob {
    pub request: LiveRequest,
    pub reply: mpsc::Sender<LiveReply>,
}

type Waker = Box<dyn Fn() + Send + Sync>;

#[derive(Default)]
struct QueueInner {
    jobs: Mutex<VecDeque<LiveJob>>,
    waker: Mutex<Option<Waker>>,
}

/// The thread-safe queue between the server thread and the editor thread.
/// Cloning shares it.
#[derive(Clone, Default)]
pub struct CommandQueue {
    inner: Arc<QueueInner>,
}

impl CommandQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// Called after each push, from the pushing thread: wakes the editor loop.
    pub fn set_waker(&self, wake: impl Fn() + Send + Sync + 'static) {
        if let Ok(mut slot) = self.inner.waker.lock() {
            *slot = Some(Box::new(wake));
        }
    }

    pub fn push(&self, job: LiveJob) {
        if let Ok(mut jobs) = self.inner.jobs.lock() {
            jobs.push_back(job);
        }
        if let Ok(slot) = self.inner.waker.lock()
            && let Some(wake) = slot.as_ref()
        {
            wake();
        }
    }

    /// Takes everything posted so far, in order.
    pub fn drain(&self) -> Vec<LiveJob> {
        self.inner
            .jobs
            .lock()
            .map(|mut jobs| jobs.drain(..).collect())
            .unwrap_or_default()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.jobs.lock().is_ok_and(|jobs| jobs.is_empty())
    }
}

// ------------------------------------------------------------ the host

/// A [`ProjectHost`] that owns nothing: it forwards to the editor through the
/// queue and waits for the answer. `Send`, so the generic MCP worker thread
/// can own it.
#[derive(Clone)]
pub struct LiveHost {
    queue: CommandQueue,
}

impl LiveHost {
    pub fn new(queue: CommandQueue) -> Self {
        Self { queue }
    }

    fn ask(&self, request: LiveRequest, timeout: Duration) -> Result<LiveReply, ToolError> {
        let (reply, answer) = mpsc::channel();
        self.queue.push(LiveJob { request, reply });
        answer.recv_timeout(timeout).map_err(|e| match e {
            mpsc::RecvTimeoutError::Timeout => ToolError::unavailable(format!(
                "the editor did not answer within {} s (is it busy with a dialog or closed?)",
                timeout.as_secs()
            )),
            mpsc::RecvTimeoutError::Disconnected => {
                ToolError::unavailable("the editor dropped the request (it is closing)")
            }
        })
    }
}

fn protocol_error() -> ToolError {
    ToolError::new(
        ErrorKind::Internal,
        "the editor answered with an unexpected message",
    )
}

impl ProjectHost for LiveHost {
    fn list_tools(&self) -> Vec<ToolInfo> {
        match self.ask(LiveRequest::ListTools, REPLY_TIMEOUT) {
            Ok(LiveReply::Tools(tools)) => tools,
            _ => Vec::new(),
        }
    }

    fn call_tool(&mut self, name: &str, args: Json) -> Result<String, ToolError> {
        self.call_tool_rich(name, args).map(|out| out.text)
    }

    fn call_tool_rich(&mut self, name: &str, args: Json) -> Result<ToolOutput, ToolError> {
        let args = if args.is_null() {
            serde_json::json!({})
        } else {
            args
        };
        // Build / export go through the host interface, like any other mode.
        if matches!(name, live_tools::BUILD_PROJECT | live_tools::EXPORT_GAME)
            && let Some(parsed) = live_tools::parse_live_call(name, args.clone())
        {
            return match parsed? {
                LiveCall::Export(request) => self.export(&request).map(ToolOutput::text),
                _ => Err(protocol_error()),
            };
        }
        let request = LiveRequest::CallTool {
            name: name.to_owned(),
            args,
        };
        match self.ask(request, REPLY_TIMEOUT)? {
            LiveReply::Tool(result) => result,
            _ => Err(protocol_error()),
        }
    }

    fn list_resources(&self) -> Vec<ResourceInfo> {
        match self.ask(LiveRequest::ListResources, REPLY_TIMEOUT) {
            Ok(LiveReply::Resources(list)) => list,
            _ => Vec::new(),
        }
    }

    fn read_resource(&self, uri: &str) -> Result<(&'static str, String), ToolError> {
        let request = LiveRequest::ReadResource {
            uri: uri.to_owned(),
        };
        match self.ask(request, REPLY_TIMEOUT)? {
            LiveReply::Resource(result) => result,
            _ => Err(protocol_error()),
        }
    }

    fn export(&mut self, request: &ExportRequest) -> Result<String, ToolError> {
        match self.ask(LiveRequest::Export(request.clone()), EXPORT_TIMEOUT)? {
            LiveReply::Export(result) => result,
            _ => Err(protocol_error()),
        }
    }
}

// ------------------------------------------------------------- the token

/// A fresh random access token: `orbit_` + 64 hex digits (256 bits).
pub fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    if getrandom::fill(&mut bytes).is_err() {
        // The OS generator failed (extremely rare): fall back to time and
        // address entropy rather than a constant, but say it is weaker.
        let seed = Instant::now();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let mut x = nanos ^ (std::ptr::addr_of!(seed) as usize as u128);
        for b in &mut bytes {
            x = x
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            *b = (x >> 64) as u8;
        }
    }
    let mut token = String::from("orbit_");
    for b in bytes {
        token.push_str(&format!("{b:02x}"));
    }
    token
}

/// Constant-time comparison (the token must not leak through timing).
fn token_matches(expected: &str, given: &str) -> bool {
    let (a, b) = (expected.as_bytes(), given.as_bytes());
    let mut diff = a.len() ^ b.len();
    for i in 0..a.len().max(b.len()) {
        diff |= usize::from(a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0));
    }
    diff == 0
}

/// The token in `Authorization: Bearer <token>`.
fn bearer(request: &Request<Incoming>) -> Option<&str> {
    let value = request.headers().get(AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    scheme.eq_ignore_ascii_case("bearer").then(|| token.trim())
}

// ------------------------------------------------------------ the server

/// What the panel shows about the server.
#[derive(Default)]
pub struct ServerStatus {
    /// MCP sessions open right now (a connected client).
    pub sessions: AtomicUsize,
    /// Requests that passed authentication.
    pub requests: AtomicU64,
    /// Requests refused (bad or missing token, not loopback).
    pub rejected: AtomicU64,
    /// Unix ms of the last authenticated request (0: none yet).
    pub last_request_ms: AtomicU64,
}

fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// Settings of a live server.
#[derive(Debug, Clone)]
pub struct LiveConfig {
    /// TCP port on 127.0.0.1 (0: let the system pick one).
    pub port: u16,
    pub token: String,
}

/// A running live server. Dropping it (or [`LiveServer::stop`]) shuts it down.
pub struct LiveServer {
    addr: SocketAddr,
    status: Arc<ServerStatus>,
    cancel: CancellationToken,
    thread: Option<JoinHandle<()>>,
}

type Body = BoxBody<Bytes, Infallible>;

fn text_response(status: StatusCode, message: &str) -> Response<Body> {
    let body = Full::new(Bytes::from(
        serde_json::json!({"error": message}).to_string(),
    ))
    .boxed();
    let mut response = Response::new(body);
    *response.status_mut() = status;
    response
        .headers_mut()
        .insert(CONTENT_TYPE, "application/json".parse().expect("static"));
    response
}

impl LiveServer {
    /// Binds `127.0.0.1:<port>` and serves until stopped. The bind happens
    /// here, so a busy port is an immediate error.
    pub fn start(queue: CommandQueue, config: LiveConfig) -> Result<Self, String> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, config.port))
            .map_err(|e| format!("cannot listen on 127.0.0.1:{}: {e}", config.port))?;
        listener
            .set_nonblocking(true)
            .map_err(|e| format!("cannot configure the listener: {e}"))?;
        let addr = listener
            .local_addr()
            .map_err(|e| format!("cannot read the listener address: {e}"))?;

        // The generic MCP worker thread; its host forwards to the editor.
        let mcp = OrbitMcpServer::spawn(move || Ok(LiveHost::new(queue)))?
            .with_instructions(LIVE_INSTRUCTIONS);

        let status = Arc::new(ServerStatus::default());
        let cancel = CancellationToken::new();
        let thread = {
            let (status, cancel) = (status.clone(), cancel.clone());
            thread::Builder::new()
                .name("orbit-mcp-http".into())
                .spawn(move || run_http(listener, mcp, config.token, status, cancel))
                .map_err(|e| format!("cannot start the server thread: {e}"))?
        };
        Ok(Self {
            addr,
            status,
            cancel,
            thread: Some(thread),
        })
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    pub fn status(&self) -> &Arc<ServerStatus> {
        &self.status
    }

    /// URL clients connect to.
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/mcp", self.addr.port())
    }

    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.cancel.cancel();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for LiveServer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn run_http(
    listener: TcpListener,
    mcp: OrbitMcpServer,
    token: String,
    status: Arc<ServerStatus>,
    cancel: CancellationToken,
) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("orbit mcp (live): cannot start the async runtime: {e}");
            return;
        }
    };
    runtime.block_on(async move {
        let listener = match tokio::net::TcpListener::from_std(listener) {
            Ok(listener) => listener,
            Err(e) => {
                eprintln!("orbit mcp (live): cannot use the listener: {e}");
                return;
            }
        };
        let sessions = Arc::new(LocalSessionManager::default());
        let config = StreamableHttpServerConfig::default()
            .with_allowed_hosts(["localhost", "127.0.0.1", "::1"])
            .with_json_response(true)
            .with_cancellation_token(cancel.child_token());
        let service = StreamableHttpService::new(move || Ok(mcp.clone()), sessions.clone(), config);
        let token: Arc<str> = Arc::from(token);

        // Keeps the "client connected" figure current.
        {
            let (status, sessions, cancel) = (status.clone(), sessions.clone(), cancel.clone());
            tokio::spawn(async move {
                while !cancel.is_cancelled() {
                    let n = sessions.sessions.read().await.len();
                    status.sessions.store(n, Ordering::Relaxed);
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
            });
        }

        loop {
            let (stream, peer) = tokio::select! {
                () = cancel.cancelled() => break,
                accepted = listener.accept() => match accepted {
                    Ok(pair) => pair,
                    Err(_) => continue,
                },
            };
            let (service, token, status) = (service.clone(), token.clone(), status.clone());
            tokio::spawn(async move {
                let handler = service_fn(move |request: Request<Incoming>| {
                    let (service, token, status) = (service.clone(), token.clone(), status.clone());
                    async move {
                        Ok::<_, Infallible>(
                            handle_request(&service, &token, &status, peer, request).await,
                        )
                    }
                });
                // Errors are the client hanging up; nothing to report.
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), handler)
                    .await;
            });
        }
    });
}

async fn handle_request(
    service: &StreamableHttpService<OrbitMcpServer, LocalSessionManager>,
    token: &str,
    status: &ServerStatus,
    peer: SocketAddr,
    request: Request<Incoming>,
) -> Response<Body> {
    // Belt and braces: the listener is bound to loopback already.
    if !peer.ip().is_loopback() {
        status.rejected.fetch_add(1, Ordering::Relaxed);
        return text_response(StatusCode::FORBIDDEN, "only local connections are accepted");
    }
    let authorized = bearer(&request).is_some_and(|given| token_matches(token, given));
    if !authorized {
        status.rejected.fetch_add(1, Ordering::Relaxed);
        let mut response = text_response(
            StatusCode::UNAUTHORIZED,
            "missing or invalid access token (Authorization: Bearer <token>)",
        );
        response
            .headers_mut()
            .insert(WWW_AUTHENTICATE, "Bearer".parse().expect("static"));
        return response;
    }
    status.requests.fetch_add(1, Ordering::Relaxed);
    status.last_request_ms.store(unix_ms(), Ordering::Relaxed);
    service.handle(request).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_long_unique_and_hex() {
        let (a, b) = (generate_token(), generate_token());
        assert_ne!(a, b);
        assert_eq!(a.len(), "orbit_".len() + 64);
        assert!(a["orbit_".len()..].chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn token_comparison_is_exact() {
        assert!(token_matches("orbit_abc", "orbit_abc"));
        assert!(!token_matches("orbit_abc", "orbit_abd"));
        assert!(!token_matches("orbit_abc", "orbit_ab"));
        assert!(!token_matches("orbit_abc", ""));
    }

    #[test]
    fn the_queue_keeps_order_and_wakes() {
        let queue = CommandQueue::new();
        let woken = Arc::new(AtomicUsize::new(0));
        let w = woken.clone();
        queue.set_waker(move || {
            w.fetch_add(1, Ordering::Relaxed);
        });
        let (tx, _rx) = mpsc::channel();
        for name in ["a", "b"] {
            queue.push(LiveJob {
                request: LiveRequest::CallTool {
                    name: name.into(),
                    args: Json::Null,
                },
                reply: tx.clone(),
            });
        }
        assert_eq!(woken.load(Ordering::Relaxed), 2);
        let jobs = queue.drain();
        assert_eq!(jobs.len(), 2);
        assert!(matches!(&jobs[0].request, LiveRequest::CallTool { name, .. } if name == "a"));
        assert!(queue.is_empty());
    }

    #[test]
    fn a_host_without_an_editor_times_out_with_a_clear_error() {
        // Nobody drains the queue: the call must fail, not hang.
        let host = LiveHost::new(CommandQueue::new());
        let err = host
            .ask(LiveRequest::ListTools, Duration::from_millis(50))
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::Unavailable);
    }
}
