//! MCP transport: the `rmcp` server handler and the stdio entry point.
//!
//! The project lives on a dedicated worker thread (a `Project` and the
//! plugin registry are not `Send`); the async handler only forwards requests
//! to it through a channel. This also serializes every call, which is the
//! same model the live mode (M4) uses with the editor's command queue.

use std::borrow::Cow;
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
    ListResourcesResult, ListToolsResult, PaginatedRequestParams, ReadResourceRequestParams,
    ReadResourceResponse, ReadResourceResult, Resource, ResourceContents, ServerCapabilities,
    ServerConfig, Tool, ToolAnnotations,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler, ServiceExt};
use serde_json::Value as Json;
use tokio::sync::oneshot;

use crate::error::ToolError;
use crate::host::{HeadlessHost, ProjectHost, ResourceInfo, ToolInfo};

const INSTRUCTIONS: &str = "\
OrbitEngine visual novel project. Typical workflow: list_scenes / read_scene to look at the story; \
create_character (import_asset for images first), create_scene, then add_dialogue, add_command \
(labels first, then jumps) and add_choice; validate; simulate with `choices` to check each branch. \
Every successful call is saved to disk and is one undo entry (undo / redo tools). Nodes are \
addressed by id; read_scene shows versions you can pass as expected_version. File access is limited \
to the project folder.";

enum Job {
    ListTools(oneshot::Sender<Vec<ToolInfo>>),
    CallTool {
        name: String,
        args: Json,
        reply: oneshot::Sender<Result<String, ToolError>>,
    },
    ListResources(oneshot::Sender<Vec<ResourceInfo>>),
    ReadResource {
        uri: String,
        reply: oneshot::Sender<Result<(&'static str, String), ToolError>>,
    },
}

/// The `rmcp` handler. Cheap to clone; all state is on the worker thread.
#[derive(Clone)]
pub struct OrbitMcpServer {
    jobs: mpsc::Sender<Job>,
}

impl OrbitMcpServer {
    /// Starts the worker thread owning the host built by `make` (built on
    /// that thread, so it does not need to be `Send`).
    pub fn spawn<H, F>(make: F) -> Result<Self, String>
    where
        H: ProjectHost + 'static,
        F: FnOnce() -> Result<H, String> + Send + 'static,
    {
        let (jobs, inbox) = mpsc::channel::<Job>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
        thread::Builder::new()
            .name("orbit-mcp-project".into())
            .spawn(move || {
                let mut host = match make() {
                    Ok(host) => {
                        let _ = ready_tx.send(Ok(()));
                        host
                    }
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return;
                    }
                };
                for job in inbox {
                    // The requester may have gone away (cancelled): ignore.
                    match job {
                        Job::ListTools(reply) => {
                            let _ = reply.send(host.list_tools());
                        }
                        Job::CallTool { name, args, reply } => {
                            let _ = reply.send(host.call_tool(&name, args));
                        }
                        Job::ListResources(reply) => {
                            let _ = reply.send(host.list_resources());
                        }
                        Job::ReadResource { uri, reply } => {
                            let _ = reply.send(host.read_resource(&uri));
                        }
                    }
                }
            })
            .map_err(|e| format!("cannot start the project thread: {e}"))?;
        ready_rx
            .recv()
            .map_err(|_| "the project thread stopped unexpectedly".to_owned())??;
        Ok(Self { jobs })
    }

    async fn ask<T>(&self, make: impl FnOnce(oneshot::Sender<T>) -> Job) -> Result<T, McpError> {
        let (tx, rx) = oneshot::channel();
        self.jobs
            .send(make(tx))
            .map_err(|_| McpError::internal_error("the project thread is gone", None))?;
        rx.await
            .map_err(|_| McpError::internal_error("the project thread dropped the request", None))
    }
}

fn to_tool(info: ToolInfo) -> Tool {
    let annotations = ToolAnnotations::with_title(info.title.clone())
        .read_only(info.read_only)
        .destructive(info.destructive)
        .idempotent(info.idempotent)
        .open_world(false);
    Tool::new(
        Cow::Owned(info.name),
        Cow::Owned(info.description),
        Arc::new(info.input_schema),
    )
    .with_title(info.title)
    .with_annotations(annotations)
}

fn to_resource(info: ResourceInfo) -> Resource {
    Resource::new(info.uri, info.name)
        .with_description(info.description)
        .with_mime_type(info.mime_type)
}

impl ServerHandler for OrbitMcpServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_server_info(Implementation::new("orbit-mcp", env!("CARGO_PKG_VERSION")))
        .with_instructions(INSTRUCTIONS)
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let infos = self.ask(Job::ListTools).await?;
        Ok(ListToolsResult::with_all_items(
            infos.into_iter().map(to_tool).collect(),
        ))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let name = request.name.to_string();
        let args = request.arguments.map_or(Json::Null, Json::Object);
        let result = self
            .ask(|reply| Job::CallTool { name, args, reply })
            .await?;
        // A refused call is a normal tool result flagged as an error, so the
        // model reads the reason and can correct itself.
        Ok(match result {
            Ok(text) => CallToolResult::success(vec![ContentBlock::text(text)]),
            Err(err) => CallToolResult::error(vec![ContentBlock::text(err.to_string())]),
        }
        .into())
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        let infos = self.ask(Job::ListResources).await?;
        Ok(ListResourcesResult::with_all_items(
            infos.into_iter().map(to_resource).collect(),
        ))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, McpError> {
        let uri = request.uri;
        let result = self
            .ask(|reply| Job::ReadResource {
                uri: uri.clone(),
                reply,
            })
            .await?;
        match result {
            Ok((mime, text)) => Ok(ReadResourceResult::new(vec![
                ResourceContents::text(text, uri).with_mime_type(mime),
            ])
            .into()),
            Err(err) => Err(McpError::resource_not_found(err.message, None)),
        }
    }
}

/// Opens `project` (a folder or a story file) and serves MCP on stdin /
/// stdout until the client disconnects. Diagnostics go to stderr only:
/// stdout carries the protocol.
pub fn serve_stdio(project: &Path) -> Result<(), String> {
    let path = project.to_owned();
    let server =
        OrbitMcpServer::spawn(move || HeadlessHost::open(&path).map_err(|e| e.to_string()))?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("cannot start the async runtime: {e}"))?;
    runtime.block_on(async move {
        let running = server
            .serve(rmcp::transport::stdio())
            .await
            .map_err(|e| format!("MCP initialization failed: {e}"))?;
        running
            .waiting()
            .await
            .map_err(|e| format!("MCP session ended with an error: {e}"))?;
        Ok(())
    })
}
