//! Live mode (M4): the MCP server that runs **inside the editor**.
//!
//! ```text
//!  client ──HTTP──▶ [server thread: tokio, auth, rmcp]
//!                        │ OrbitMcpServer worker thread (one call at a time)
//!                        ▼
//!                  a ProjectHost ──▶ the editor's queue ──▶ editor thread
//!                        ▲                                      │ runs the tool
//!                        └────────────── reply (channel) ◀──────┘ on the live Project
//! ```
//!
//! * The server only ever listens on **127.0.0.1**; every request needs the
//!   access token (`Authorization: Bearer <token>`); `Host` is checked by
//!   `rmcp` against DNS rebinding.
//! * It never touches a project: its [`ProjectHost`](crate::host::ProjectHost)
//!   forwards every request to the editor thread (the `Project`, the
//!   registry and the GPU are not `Send`). In the editor, that host is the
//!   `orbit.mcp` plugin's bridge to the editor's request queue
//!   (`orbit_editor_api::mcp::McpQueue`).
//!
//! The editor links this crate without the `server` feature: only
//! [`generate_token`] is there then. `LiveServer` (tokio, hyper, rmcp) is
//! compiled with the feature, by the `orbit.mcp` plugin.

use std::time::Instant;

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

#[cfg(feature = "server")]
pub use http::{LiveConfig, LiveServer, ServerStatus};

/// The HTTP server (feature `server`).
#[cfg(feature = "server")]
mod http {
    use std::convert::Infallible;
    use std::net::{Ipv4Addr, SocketAddr, TcpListener};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    use std::thread::{self, JoinHandle};
    use std::time::Duration;

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
    use tokio_util::sync::CancellationToken;

    use crate::host::{ProjectHost, ToolInfo};
    use crate::server::{LIVE_INSTRUCTIONS, OrbitMcpServer};

    /// Constant-time comparison (the token must not leak through timing).
    pub(super) fn token_matches(expected: &str, given: &str) -> bool {
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
        ///
        /// `make_host` builds, on the MCP worker thread, the host every request
        /// goes to; `tool_list` answers `tools/list` from the server thread
        /// (`None`: ask the host).
        pub fn start<H, F>(
            make_host: F,
            tool_list: impl Fn() -> Option<Vec<ToolInfo>> + Send + Sync + 'static,
            config: LiveConfig,
        ) -> Result<Self, String>
        where
            H: ProjectHost + 'static,
            F: FnOnce() -> Result<H, String> + Send + 'static,
        {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, config.port))
                .map_err(|e| format!("cannot listen on 127.0.0.1:{}: {e}", config.port))?;
            listener
                .set_nonblocking(true)
                .map_err(|e| format!("cannot configure the listener: {e}"))?;
            let addr = listener
                .local_addr()
                .map_err(|e| format!("cannot read the listener address: {e}"))?;

            // The generic MCP worker thread; its host forwards to the editor.
            let mcp = OrbitMcpServer::spawn(make_host)?
                .with_instructions(LIVE_INSTRUCTIONS)
                .with_tool_list(tool_list);

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

    #[cfg(feature = "server")]
    #[test]
    fn token_comparison_is_exact() {
        use super::http::token_matches;
        assert!(token_matches("orbit_abc", "orbit_abc"));
        assert!(!token_matches("orbit_abc", "orbit_abd"));
        assert!(!token_matches("orbit_abc", "orbit_ab"));
        assert!(!token_matches("orbit_abc", ""));
    }
}
