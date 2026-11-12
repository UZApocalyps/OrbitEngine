//! The live MCP bridge between the editor and the plugin that serves it on
//! the network (`orbit.mcp`).
//!
//! ```text
//!  client ──HTTP──▶ [plugin: McpTransport, tokio / hyper / rmcp]
//!                        │ McpQueue::ask (any thread)
//!                        ▼
//!                  McpQueue ──▶ editor thread: drains it between two frames,
//!                        ▲      runs the tool on the open Project
//!                        └──── reply (channel)
//! ```
//!
//! The editor owns the queue and runs every request (tools, resources,
//! export) itself: the transport never touches a project. The editor thus
//! links no async runtime and no network stack; without the plugin, the
//! tools still run for plugins (`EditorContext::call_tool`).
//!
//! Everything here is plain `std` data (JSON travels as text): the plugin
//! builds its MCP crate with other features than the editor, so no type of
//! that crate may cross the boundary.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

/// How long a request waits for the editor to answer (a sleeping or blocked
/// editor must not hang the client for ever).
pub const REPLY_TIMEOUT: Duration = Duration::from_secs(60);
/// Builds and exports take longer.
pub const EXPORT_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// How long a posted request may wait for the editor thread to pick it up
/// before the editor counts as busy (a native file dialog blocks its thread,
/// for example). The request is then withdrawn, never run, and the client
/// gets a clear `unavailable` error instead of a silent timeout.
pub const PICKUP_GRACE: Duration = Duration::from_secs(5);

/// A tool as offered to clients.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpTool {
    pub name: String,
    pub title: String,
    pub description: String,
    /// The JSON schema of the arguments (a JSON object, as text).
    pub input_schema: String,
    pub read_only: bool,
    pub destructive: bool,
    pub idempotent: bool,
}

/// A resource offered to clients.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpResource {
    pub uri: String,
    pub name: String,
    pub description: String,
    pub mime_type: String,
}

/// A refused request: `kind` is the stable error code clients branch on
/// (`not_found`, `version_conflict`, `unavailable`...).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpError {
    pub kind: String,
    pub message: String,
}

impl McpError {
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self {
            kind: "unavailable".into(),
            message: message.into(),
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            kind: "internal_error".into(),
            message: message.into(),
        }
    }
}

/// What a tool call returns: text, plus pictures (a screenshot).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct McpOutput {
    pub text: String,
    /// `(mime type, base64 data)`.
    pub images: Vec<(String, String)>,
}

/// A request to build or export the game.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpExport {
    /// `true` for a bare build (pak of assets), `false` for a full export.
    pub build_only: bool,
    pub target: Option<String>,
    pub profile: Option<String>,
    /// Output folder, relative to the project folder.
    pub out_dir: Option<String>,
}

/// What the transport asks of the editor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum McpRequest {
    ListTools,
    /// `args`: a JSON value, as text.
    CallTool {
        name: String,
        args: String,
    },
    ListResources,
    ReadResource {
        uri: String,
    },
    Export(McpExport),
}

/// The editor's answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum McpReply {
    Tools(Vec<McpTool>),
    Tool(Result<McpOutput, McpError>),
    Resources(Vec<McpResource>),
    /// `(mime type, text)`.
    Resource(Result<(String, String), McpError>),
    Export(Result<String, McpError>),
}

/// A request and the way back.
#[derive(Debug)]
pub struct McpJob {
    pub request: McpRequest,
    pub reply: mpsc::Sender<McpReply>,
}

type WakeFn = Box<dyn Fn() + Send + Sync>;

#[derive(Default)]
struct QueueInner {
    jobs: Mutex<VecDeque<(u64, McpJob)>>,
    next_id: AtomicU64,
    waker: Mutex<Option<WakeFn>>,
    /// The editor's tool list, published from its thread: `tools/list`
    /// never needs the editor thread (see [`McpQueue::publish_tools`]).
    tools: Mutex<Option<Arc<Vec<McpTool>>>>,
    /// Why the editor cannot answer right now, when it knows (shown to the
    /// client in the "editor busy" error).
    busy: Mutex<Option<String>>,
}

/// The thread-safe queue between the transport's threads and the editor
/// thread. Cloning shares it.
#[derive(Clone, Default)]
pub struct McpQueue {
    inner: Arc<QueueInner>,
}

impl std::fmt::Debug for McpQueue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpQueue").finish_non_exhaustive()
    }
}

impl McpQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// Called after each push, from the pushing thread: wakes the editor loop.
    pub fn set_waker(&self, wake: impl Fn() + Send + Sync + 'static) {
        if let Ok(mut slot) = self.inner.waker.lock() {
            *slot = Some(Box::new(wake));
        }
    }

    /// Posts a job and wakes the editor; returns its ticket (see
    /// [`McpQueue::withdraw`]).
    pub fn push(&self, job: McpJob) -> u64 {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut jobs) = self.inner.jobs.lock() {
            jobs.push_back((id, job));
        }
        self.wake();
        id
    }

    /// Asks the editor loop for a frame (the window draws on demand).
    pub fn wake(&self) {
        if let Ok(slot) = self.inner.waker.lock()
            && let Some(wake) = slot.as_ref()
        {
            wake();
        }
    }

    /// Takes back a job the editor has not picked up yet. `false`: it was
    /// already taken (it runs or ran).
    pub fn withdraw(&self, id: u64) -> bool {
        let Ok(mut jobs) = self.inner.jobs.lock() else {
            return false;
        };
        let before = jobs.len();
        jobs.retain(|(job_id, _)| *job_id != id);
        jobs.len() != before
    }

    /// Takes everything posted so far, in order (editor thread).
    pub fn drain(&self) -> Vec<McpJob> {
        self.inner
            .jobs
            .lock()
            .map(|mut jobs| jobs.drain(..).map(|(_, job)| job).collect())
            .unwrap_or_default()
    }

    /// The editor publishes the tool list clients see (already filtered by
    /// the experimental switches). `tools/list` is then answered from the
    /// transport's thread, even while the editor thread is asleep or blocked.
    pub fn publish_tools(&self, tools: Vec<McpTool>) {
        if let Ok(mut slot) = self.inner.tools.lock() {
            *slot = Some(Arc::new(tools));
        }
    }

    /// The published tool list.
    pub fn tools(&self) -> Option<Arc<Vec<McpTool>>> {
        self.inner.tools.lock().ok()?.clone()
    }

    /// The editor says why it cannot answer for a while (`None`: it can).
    pub fn set_busy(&self, reason: Option<String>) {
        if let Ok(mut slot) = self.inner.busy.lock() {
            *slot = reason;
        }
    }

    pub fn busy_reason(&self) -> Option<String> {
        self.inner.busy.lock().ok().and_then(|r| r.clone())
    }

    pub fn is_empty(&self) -> bool {
        self.inner.jobs.lock().is_ok_and(|jobs| jobs.is_empty())
    }

    /// Posts `request` and waits for the answer ([`REPLY_TIMEOUT`], or
    /// [`EXPORT_TIMEOUT`] for an export). Call it from a transport thread,
    /// never from the editor thread.
    pub fn ask(&self, request: McpRequest) -> Result<McpReply, McpError> {
        let timeout = if matches!(request, McpRequest::Export(_)) {
            EXPORT_TIMEOUT
        } else {
            REPLY_TIMEOUT
        };
        self.ask_with_grace(request, timeout, PICKUP_GRACE)
    }

    /// Posts `request` and waits for the answer. When the editor thread has
    /// not even picked the job up after `grace` (it is blocked: a native file
    /// dialog, a frozen window), the job is withdrawn — it will never run —
    /// and the caller gets an "editor busy" error at once rather than after
    /// `timeout`.
    pub fn ask_with_grace(
        &self,
        request: McpRequest,
        timeout: Duration,
        grace: Duration,
    ) -> Result<McpReply, McpError> {
        let (reply, answer) = mpsc::channel();
        let id = self.push(McpJob { request, reply });
        let started = Instant::now();
        let mut picked = false;
        loop {
            let slice = Duration::from_millis(200).min(timeout.saturating_sub(started.elapsed()));
            match answer.recv_timeout(slice) {
                Ok(reply) => return Ok(reply),
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(McpError::unavailable(
                        "the editor dropped the request (it is closing)",
                    ));
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            let elapsed = started.elapsed();
            if !picked && elapsed >= grace {
                if self.withdraw(id) {
                    let reason = self.busy_reason().unwrap_or_else(|| {
                        "its window is not processing events (a native dialog such as a file \
                         picker may be open, or the window is frozen)"
                            .to_owned()
                    });
                    return Err(McpError::unavailable(format!(
                        "editor busy: {reason}. The request was not run; close the dialog in \
                         the editor and try again."
                    )));
                }
                picked = true; // the editor runs it: wait for the answer
            }
            if elapsed >= timeout {
                return Err(McpError::unavailable(format!(
                    "the editor did not answer within {} s (is it busy with a dialog or closed?)",
                    timeout.as_secs()
                )));
            }
            if !picked {
                // A lost wake-up must not cost the whole grace period.
                self.wake();
            }
        }
    }
}

/// Live figures of a running server, for the panel and the status bar.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct McpServerStats {
    /// MCP sessions open right now.
    pub sessions: usize,
    /// Requests refused (bad or missing token, not loopback).
    pub rejected: u64,
    /// Unix ms of the last authenticated request (0: none yet).
    pub last_request_ms: u64,
}

/// The network side of the live MCP server, provided by a plugin (the
/// official one is `orbit.mcp`) through `PluginHost::mcp_transport`. The
/// editor calls it on its own thread; requests come back through the
/// [`McpQueue`] it is given.
pub trait McpTransport {
    /// Starts serving `queue` on `127.0.0.1:<port>` (0: any free port) for
    /// clients that send `token`. Returns the port really bound. The bind
    /// happens here: a busy port is an immediate error.
    fn start(&self, queue: McpQueue, port: u16, token: &str) -> Result<u16, String>;
    /// Stops the server (clients are disconnected). Nothing when stopped.
    fn stop(&self);
    /// Figures of the running server (`None` when stopped).
    fn stats(&self) -> Option<McpServerStats>;
}

/// A transport whose every call goes through the plugin's guard (a panic
/// fails the plugin instead of unwinding into the editor). Built in the
/// plugin's copy of this crate by `PluginHost::mcp_transport`.
pub(crate) struct GuardedTransport {
    pub(crate) inner: Box<dyn McpTransport>,
    pub(crate) guard: orbit_plugin_api::PluginGuard,
}

impl McpTransport for GuardedTransport {
    fn start(&self, queue: McpQueue, port: u16, token: &str) -> Result<u16, String> {
        let guard = &self.guard;
        guard.call(
            || {
                Err(format!(
                    "plugin `{}` failed: {}",
                    guard.plugin(),
                    guard.failure().unwrap_or_default()
                ))
            },
            || self.inner.start(queue, port, token),
        )
    }

    fn stop(&self) {
        self.guard.call(|| (), || self.inner.stop());
    }

    fn stats(&self) -> Option<McpServerStats> {
        self.guard.call(|| None, || self.inner.stats())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;

    use super::*;

    fn call(name: &str) -> McpRequest {
        McpRequest::CallTool {
            name: name.into(),
            args: "null".into(),
        }
    }

    #[test]
    fn the_queue_keeps_order_and_wakes() {
        let queue = McpQueue::new();
        let woken = Arc::new(AtomicUsize::new(0));
        let w = woken.clone();
        queue.set_waker(move || {
            w.fetch_add(1, Ordering::Relaxed);
        });
        let (tx, _rx) = mpsc::channel();
        for name in ["a", "b"] {
            queue.push(McpJob {
                request: call(name),
                reply: tx.clone(),
            });
        }
        assert_eq!(woken.load(Ordering::Relaxed), 2);
        let jobs = queue.drain();
        assert_eq!(jobs.len(), 2);
        assert!(matches!(&jobs[0].request, McpRequest::CallTool { name, .. } if name == "a"));
        assert!(queue.is_empty());
    }

    #[test]
    fn a_queue_without_an_editor_times_out_with_a_clear_error() {
        // Nobody drains the queue: the call must fail, not hang.
        let queue = McpQueue::new();
        let err = queue
            .ask_with_grace(
                McpRequest::ListTools,
                Duration::from_millis(50),
                Duration::from_secs(5),
            )
            .unwrap_err();
        assert_eq!(err.kind, "unavailable");
    }

    #[test]
    fn a_blocked_editor_gets_a_quick_busy_error_and_the_job_is_withdrawn() {
        let queue = McpQueue::new();
        queue.set_busy(Some("a file dialog is open".into()));
        let started = Instant::now();
        let err = queue
            .ask_with_grace(
                call("add_dialogue"),
                Duration::from_secs(60),
                Duration::from_millis(100),
            )
            .unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(err.kind, "unavailable");
        assert!(
            err.message.contains("editor busy: a file dialog is open"),
            "{}",
            err.message
        );
        // Never run later, when the editor comes back.
        assert!(queue.is_empty());
    }

    #[test]
    fn an_answer_from_the_editor_thread_comes_back() {
        let queue = McpQueue::new();
        let editor = queue.clone();
        let worker = std::thread::spawn(move || {
            loop {
                if let Some(job) = editor.drain().pop() {
                    let _ = job.reply.send(McpReply::Tools(Vec::new()));
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        });
        let reply = queue.ask(McpRequest::ListTools).unwrap();
        assert_eq!(reply, McpReply::Tools(Vec::new()));
        worker.join().unwrap();
    }

    #[test]
    fn the_tool_list_is_read_without_the_editor() {
        let queue = McpQueue::new();
        assert!(queue.tools().is_none());
        queue.publish_tools(vec![McpTool {
            name: "list_scenes".into(),
            title: "List scenes".into(),
            description: String::new(),
            input_schema: "{}".into(),
            read_only: true,
            destructive: false,
            idempotent: true,
        }]);
        assert_eq!(queue.tools().map(|t| t.len()), Some(1));
        assert!(queue.is_empty());
    }
}
