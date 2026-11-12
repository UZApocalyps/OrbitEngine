//! The network side of the editor's live MCP server: `orbit_mcp`'s HTTP
//! server (tokio, hyper, rmcp) in front of the editor's request queue.
//!
//! The editor runs every request itself (`McpQueue`, drained on its thread);
//! this side only speaks the protocol. The two exchange plain `std` data
//! (`orbit_editor_sdk::mcp`), converted here from and to `orbit_mcp`'s types:
//! this plugin builds `orbit_mcp` with its `server` feature, the editor
//! without, so no `orbit_mcp` value may cross the boundary.

use std::sync::Mutex;
use std::sync::atomic::Ordering;

use orbit_editor_sdk::mcp::{
    McpError, McpExport, McpOutput, McpQueue, McpReply, McpRequest, McpServerStats, McpTool,
    McpTransport,
};
use orbit_mcp::host::{ExportRequest, ProjectHost, ResourceInfo, ToolInfo, ToolOutput};
use orbit_mcp::live::{LiveConfig, LiveServer};
use orbit_mcp::live_tools::{self, LiveCall};
use orbit_mcp::{ErrorKind, ToolError};
use serde_json::Value as Json;

/// The live HTTP server, started and stopped by the editor.
#[derive(Default)]
pub struct HttpTransport {
    server: Mutex<Option<LiveServer>>,
}

impl HttpTransport {
    pub fn new() -> Self {
        Self::default()
    }
}

impl McpTransport for HttpTransport {
    fn start(&self, queue: McpQueue, port: u16, token: &str) -> Result<u16, String> {
        let mut slot = self
            .server
            .lock()
            .map_err(|_| "the MCP server state is poisoned".to_owned())?;
        if let Some(server) = slot.take() {
            server.stop();
        }
        let tools = queue.clone();
        let server = LiveServer::start(
            move || Ok(BridgeHost { queue }),
            move || {
                tools
                    .tools()
                    .map(|list| list.iter().map(tool_from_mcp).collect())
            },
            LiveConfig {
                port,
                token: token.to_owned(),
            },
        )?;
        let port = server.port();
        *slot = Some(server);
        Ok(port)
    }

    fn stop(&self) {
        let server = self.server.lock().ok().and_then(|mut slot| slot.take());
        if let Some(server) = server {
            server.stop();
        }
    }

    fn stats(&self) -> Option<McpServerStats> {
        let slot = self.server.lock().ok()?;
        let status = slot.as_ref()?.status();
        Some(McpServerStats {
            sessions: status.sessions.load(Ordering::Relaxed),
            rejected: status.rejected.load(Ordering::Relaxed),
            last_request_ms: status.last_request_ms.load(Ordering::Relaxed),
        })
    }
}

// ------------------------------------------------------------- the host

/// The [`ProjectHost`] of the server: forwards every request to the editor
/// thread through the queue and waits for the answer.
struct BridgeHost {
    queue: McpQueue,
}

fn protocol_error() -> ToolError {
    ToolError::new(
        ErrorKind::Internal,
        "the editor answered with an unexpected message",
    )
}

impl ProjectHost for BridgeHost {
    fn list_tools(&self) -> Vec<ToolInfo> {
        if let Some(tools) = self.queue.tools() {
            return tools.iter().map(tool_from_mcp).collect();
        }
        match self.queue.ask(McpRequest::ListTools) {
            Ok(McpReply::Tools(tools)) => tools.iter().map(tool_from_mcp).collect(),
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
        let request = McpRequest::CallTool {
            name: name.to_owned(),
            args: args.to_string(),
        };
        match self.queue.ask(request).map_err(error_from_mcp)? {
            McpReply::Tool(result) => result.map(output_from_mcp).map_err(error_from_mcp),
            _ => Err(protocol_error()),
        }
    }

    fn list_resources(&self) -> Vec<ResourceInfo> {
        match self.queue.ask(McpRequest::ListResources) {
            Ok(McpReply::Resources(list)) => list
                .into_iter()
                .map(|r| ResourceInfo {
                    uri: r.uri,
                    name: r.name,
                    description: r.description,
                    mime_type: static_mime(&r.mime_type),
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    fn read_resource(&self, uri: &str) -> Result<(&'static str, String), ToolError> {
        let request = McpRequest::ReadResource {
            uri: uri.to_owned(),
        };
        match self.queue.ask(request).map_err(error_from_mcp)? {
            McpReply::Resource(result) => result
                .map(|(mime, text)| (static_mime(&mime), text))
                .map_err(error_from_mcp),
            _ => Err(protocol_error()),
        }
    }

    fn export(&mut self, request: &ExportRequest) -> Result<String, ToolError> {
        let request = McpRequest::Export(McpExport {
            build_only: request.build_only,
            target: request.target.clone(),
            profile: request.profile.clone(),
            out_dir: request.out_dir.clone(),
        });
        match self.queue.ask(request).map_err(error_from_mcp)? {
            McpReply::Export(result) => result.map_err(error_from_mcp),
            _ => Err(protocol_error()),
        }
    }
}

// -------------------------------------------------------- the conversions

fn tool_from_mcp(tool: &McpTool) -> ToolInfo {
    let input_schema = match serde_json::from_str::<Json>(&tool.input_schema) {
        Ok(Json::Object(map)) => map,
        _ => serde_json::Map::new(),
    };
    ToolInfo {
        name: tool.name.clone(),
        title: tool.title.clone(),
        description: tool.description.clone(),
        input_schema,
        read_only: tool.read_only,
        destructive: tool.destructive,
        idempotent: tool.idempotent,
    }
}

fn error_from_mcp(error: McpError) -> ToolError {
    let kind = ErrorKind::from_code(&error.kind).unwrap_or(ErrorKind::Internal);
    ToolError::new(kind, error.message)
}

fn output_from_mcp(output: McpOutput) -> ToolOutput {
    ToolOutput {
        text: output.text,
        images: output.images,
    }
}

/// The MIME types the editor's resources use (`ResourceInfo` wants a
/// `'static` one).
fn static_mime(mime: &str) -> &'static str {
    match mime {
        "application/json" => "application/json",
        "text/markdown" => "text/markdown",
        "text/html" => "text/html",
        _ => "text/plain",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_kinds_survive_the_round_trip() {
        for kind in [
            ErrorKind::InvalidArguments,
            ErrorKind::NotFound,
            ErrorKind::VersionConflict,
            ErrorKind::Refused,
            ErrorKind::OutsideProject,
            ErrorKind::Io,
            ErrorKind::Plugin,
            ErrorKind::Internal,
            ErrorKind::Unavailable,
        ] {
            let error = error_from_mcp(McpError {
                kind: kind.as_str().to_owned(),
                message: "m".into(),
            });
            assert_eq!(error.kind, kind);
        }
    }

    #[test]
    fn a_tool_schema_comes_back_as_an_object() {
        let tool = tool_from_mcp(&McpTool {
            name: "read_scene".into(),
            title: "Read scene".into(),
            description: String::new(),
            input_schema: r#"{"type":"object","properties":{"scene":{"type":"string"}}}"#.into(),
            read_only: true,
            destructive: false,
            idempotent: true,
        });
        assert_eq!(tool.input_schema["type"], "object");
        assert!(tool.input_schema["properties"]["scene"].is_object());
    }
}
