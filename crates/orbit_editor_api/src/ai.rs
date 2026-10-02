//! State shared between the "Assistant IA" panel (a plugin: egui, no network)
//! and the host that runs the live MCP server (M4).
//!
//! Same shape as the preview: the host publishes the server status and the
//! journal of what the AI did; the panel reads them and posts
//! [`AiCommand`]s the host carries out between two frames.

use std::collections::VecDeque;
use std::time::Instant;

use orbit_core::NodeId;

/// Default port of the live MCP server.
pub const DEFAULT_PORT: u16 = 7878;
/// Entries kept in the journal.
pub const LOG_CAPACITY: usize = 300;

/// Where the live server is in its life.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum AiServerStatus {
    /// Off (the default: nothing listens).
    #[default]
    Stopped,
    /// Listening on 127.0.0.1, no client has an open session.
    Listening,
    /// At least one client has an open session.
    Connected { clients: usize },
    /// Could not start (port busy...).
    Failed(String),
}

impl AiServerStatus {
    pub fn is_running(&self) -> bool {
        matches!(self, Self::Listening | Self::Connected { .. })
    }
}

/// One tool call of the AI, as shown in the journal.
#[derive(Clone, Debug, PartialEq)]
pub struct AiLogEntry {
    pub id: u64,
    /// Unix time of the call, in milliseconds.
    pub time_ms: u64,
    pub tool: String,
    /// Arguments, shortened to one line.
    pub args: String,
    /// Result (or error), shortened to one line.
    pub result: String,
    pub ok: bool,
    pub duration_ms: u64,
    /// Nodes the call created, changed or removed, in first-touch order.
    pub touched: Vec<NodeId>,
    /// The call is one undo entry (an edit that went through).
    pub undoable: bool,
}

/// What the panel asks of the host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AiCommand {
    /// Starts the server on `AiState::port` with the current token.
    Start,
    Stop,
    /// A new token; the old one stops working at once.
    RegenerateToken,
    /// Undoes the last AI edit, if it is still the last thing in the history.
    UndoLast,
    ClearLog,
}

#[derive(Debug)]
pub struct AiState {
    // ---- published by the host
    pub status: AiServerStatus,
    /// Address clients use, while the server runs.
    pub url: Option<String>,
    /// Requests refused so far (bad token...).
    pub rejected: u64,
    /// What the AI did, oldest first.
    pub log: VecDeque<AiLogEntry>,
    /// The AI changed the project a moment ago (drives the discreet
    /// indicator in the panel and the status bar).
    pub last_edit: Option<Instant>,
    /// The last thing in the undo history is an AI edit.
    pub can_undo_last: bool,
    /// Another process (headless `orbit mcp`, another editor) holds the
    /// project: editing from two places overwrites silently.
    pub lock_warning: Option<String>,

    // ---- settings (persisted by the host, except `status`)
    pub port: u16,
    /// Access token (clients send it as `Authorization: Bearer <token>`).
    pub token: String,

    // ---- set by the panel
    pub commands: Vec<AiCommand>,
}

impl Default for AiState {
    fn default() -> Self {
        Self {
            status: AiServerStatus::Stopped,
            url: None,
            rejected: 0,
            log: VecDeque::new(),
            last_edit: None,
            can_undo_last: false,
            lock_warning: None,
            port: DEFAULT_PORT,
            token: String::new(),
            commands: Vec::new(),
        }
    }
}

impl AiState {
    pub fn command(&mut self, command: AiCommand) {
        self.commands.push(command);
    }

    /// Adds an entry, dropping the oldest ones beyond [`LOG_CAPACITY`].
    pub fn push_log(&mut self, entry: AiLogEntry) {
        self.log.push_back(entry);
        while self.log.len() > LOG_CAPACITY {
            self.log.pop_front();
        }
    }

    /// The AI edited the project in the last `seconds`.
    pub fn edited_within(&self, seconds: f32) -> bool {
        self.last_edit
            .is_some_and(|t| t.elapsed().as_secs_f32() < seconds)
    }

    /// `claude mcp add` command line for this server.
    pub fn claude_command(&self, port: u16) -> String {
        format!(
            "claude mcp add --transport http orbit http://127.0.0.1:{port}/mcp --header \"Authorization: Bearer {}\"",
            self.token
        )
    }

    /// `.mcp.json` snippet for this server.
    pub fn mcp_json(&self, port: u16) -> String {
        format!(
            "{{\n  \"mcpServers\": {{\n    \"orbit\": {{\n      \"type\": \"http\",\n      \"url\": \"http://127.0.0.1:{port}/mcp\",\n      \"headers\": {{\n        \"Authorization\": \"Bearer {}\"\n      }}\n    }}\n  }}\n}}\n",
            self.token
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: u64) -> AiLogEntry {
        AiLogEntry {
            id,
            time_ms: 0,
            tool: "add_dialogue".into(),
            args: String::new(),
            result: String::new(),
            ok: true,
            duration_ms: 1,
            touched: Vec::new(),
            undoable: true,
        }
    }

    #[test]
    fn the_journal_is_bounded() {
        let mut ai = AiState::default();
        for id in 0..(LOG_CAPACITY as u64 + 10) {
            ai.push_log(entry(id));
        }
        assert_eq!(ai.log.len(), LOG_CAPACITY);
        assert_eq!(ai.log.front().map(|e| e.id), Some(10));
    }

    #[test]
    fn the_config_snippets_carry_the_token_and_the_port() {
        let ai = AiState {
            token: "orbit_secret".into(),
            ..AiState::default()
        };
        let line = ai.claude_command(1234);
        assert!(line.contains("--transport http"));
        assert!(line.contains("http://127.0.0.1:1234/mcp"));
        assert!(line.contains("Authorization: Bearer orbit_secret"));
        let json = ai.mcp_json(1234);
        assert!(json.contains("\"Authorization\": \"Bearer orbit_secret\""));
        assert!(json.contains("\"url\": \"http://127.0.0.1:1234/mcp\""));
    }
}
