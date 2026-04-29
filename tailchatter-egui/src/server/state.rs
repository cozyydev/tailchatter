use std::collections::{HashMap, VecDeque};
use tokio::sync::mpsc;

use crate::protocol::{ChatLine, ServerMsg};

use super::format_for_mode;

pub type SessionId = u32;

/// Client connection mode determines serialization format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientMode {
    /// Human-readable plain text (telnet/nc clients).
    Plain,
    /// JSON serialization (GUI clients).
    Json,
}

/// A connected client session.
#[derive(Debug)]
pub struct Session {
    pub id: SessionId,
    pub nick: Option<String>,
    pub mode: ClientMode,
    pub tx: mpsc::UnboundedSender<String>,
}

/// Shared server state holding all sessions and message history.
pub struct ChatState {
    pub sessions: HashMap<SessionId, Session>,
    pub history: VecDeque<ChatLine>,
    pub dm_history: HashMap<(String, String), VecDeque<ChatLine>>,
    pub max_history: usize,
}

impl ChatState {
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
            history: VecDeque::with_capacity(1000),
            dm_history: HashMap::new(),
            max_history: 500,
        }
    }

    pub fn add_session(&mut self, session: Session) {
        self.sessions.insert(session.id, session);
    }

    /// Send a formatted message to a specific session.
    pub fn send_to(&self, session_id: SessionId, msg: ServerMsg) {
        if let Some(session) = self.sessions.get(&session_id) {
            let line = format_for_mode(&msg, session.mode);
            let _ = session.tx.send(line);
        }
    }

    /// Broadcast a message to all identified sessions.
    pub fn broadcast(&self, msg: &ServerMsg) {
        for session in self.sessions.values() {
            if session.nick.is_some() {
                let line = format_for_mode(msg, session.mode);
                let _ = session.tx.send(line);
            }
        }
    }

    /// Broadcast a chat message to everyone except the sender.
    pub fn broadcast_except(&self, sender_id: SessionId, msg: &ServerMsg) {
        for (id, session) in &self.sessions {
            if session.nick.is_some() && *id != sender_id {
                let line = format_for_mode(msg, session.mode);
                let _ = session.tx.send(line);
            }
        }
    }

    pub fn broadcast_presence(&self) {
        let msg = ServerMsg::Presence {
            online: self.online_count(),
            users: self.nick_list(),
        };
        self.broadcast(&msg);
    }

    pub fn push_history(&mut self, line: ChatLine) {
        if self.history.len() >= self.max_history {
            self.history.pop_front();
        }
        self.history.push_back(line);
    }

    pub fn push_dm_history(&mut self, from: &str, to: &str, line: ChatLine) {
        let key = dm_key(from, to);
        let entry = self.dm_history.entry(key).or_default();
        if entry.len() >= self.max_history {
            entry.pop_front();
        }
        entry.push_back(line);
    }

    pub fn get_dm_history(&self, a: &str, b: &str) -> Vec<ChatLine> {
        let key = dm_key(a, b);
        self.dm_history
            .get(&key)
            .map(|h| h.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub fn nick_in_use(&self, nick: &str) -> bool {
        self.sessions
            .values()
            .any(|s| s.nick.as_deref() == Some(nick))
    }

    pub fn find_session_id_by_nick(&self, nick: &str) -> Option<SessionId> {
        self.sessions
            .iter()
            .find(|(_, s)| s.nick.as_deref() == Some(nick))
            .map(|(id, _)| *id)
    }

    pub fn online_count(&self) -> usize {
        self.sessions
            .values()
            .filter(|s| s.nick.is_some())
            .count()
    }

    pub fn nick_list(&self) -> Vec<String> {
        self.sessions
            .values()
            .filter_map(|s| s.nick.clone())
            .collect()
    }
}

/// Create a canonical key for DM history (alphabetical order).
fn dm_key(a: &str, b: &str) -> (String, String) {
    if a < b {
        (a.to_string(), b.to_string())
    } else {
        (b.to_string(), a.to_string())
    }
}
