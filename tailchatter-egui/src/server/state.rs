use std::collections::{HashMap, VecDeque};
use tokio::sync::{mpsc, oneshot};

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

/// A connected client session (held by the actor).
#[derive(Debug)]
pub struct Session {
    pub id: SessionId,
    pub nick: Option<String>,
    pub mode: ClientMode,
    pub tx: mpsc::UnboundedSender<String>,
}

/// Commands sent to the state actor from client handler tasks.
pub enum StateCmd {
    /// Register a new session (before identification).
    AddSession {
        session: Session,
    },
    /// Remove a session on disconnect.
    RemoveSession {
        session_id: SessionId,
    },
    /// Attempt to identify a session with a nick.
    Identify {
        session_id: SessionId,
        nick: String,
        mode: ClientMode,
        reply: oneshot::Sender<Result<IdentifyOk, String>>,
    },
    /// Broadcast a group chat message from a session.
    Chat {
        session_id: SessionId,
        body: String,
    },
    /// Send a DM from one session to another user by nick.
    Dm {
        session_id: SessionId,
        to: String,
        body: String,
    },
    /// Request DM history between caller and a partner.
    GetDmHistory {
        session_id: SessionId,
        partner: String,
    },
    /// Request presence/who list.
    Who {
        session_id: SessionId,
    },
    /// Send an error message to a session.
    SendError {
        session_id: SessionId,
        body: String,
    },
}

/// Successful identification response data.
#[allow(dead_code)]
pub struct IdentifyOk {
    pub nick: String,
    pub room: String,
    pub online: usize,
}

/// Handle to communicate with the state actor.
#[derive(Clone)]
pub struct StateHandle {
    pub tx: mpsc::UnboundedSender<StateCmd>,
}

impl StateHandle {
    pub fn add_session(&self, session: Session) {
        let _ = self.tx.send(StateCmd::AddSession { session });
    }

    pub fn remove_session(&self, session_id: SessionId) {
        let _ = self.tx.send(StateCmd::RemoveSession { session_id });
    }

    pub async fn identify(
        &self,
        session_id: SessionId,
        nick: String,
        mode: ClientMode,
    ) -> Result<IdentifyOk, String> {
        let (reply_tx, reply_rx) = oneshot::channel();
        let _ = self.tx.send(StateCmd::Identify {
            session_id,
            nick,
            mode,
            reply: reply_tx,
        });
        reply_rx.await.unwrap_or(Err("Actor dropped".into()))
    }

    pub fn chat(&self, session_id: SessionId, body: String) {
        let _ = self.tx.send(StateCmd::Chat { session_id, body });
    }

    pub fn dm(&self, session_id: SessionId, to: String, body: String) {
        let _ = self.tx.send(StateCmd::Dm {
            session_id,
            to,
            body,
        });
    }

    pub fn get_dm_history(&self, session_id: SessionId, partner: String) {
        let _ = self.tx.send(StateCmd::GetDmHistory {
            session_id,
            partner,
        });
    }

    pub fn who(&self, session_id: SessionId) {
        let _ = self.tx.send(StateCmd::Who { session_id });
    }
}

/// Internal server state — owned exclusively by the actor task (no mutex needed).
struct ChatState {
    sessions: HashMap<SessionId, Session>,
    history: VecDeque<ChatLine>,
    dm_history: HashMap<(String, String), VecDeque<ChatLine>>,
    max_history: usize,
}

impl ChatState {
    fn new() -> Self {
        Self {
            sessions: HashMap::new(),
            history: VecDeque::with_capacity(1000),
            dm_history: HashMap::new(),
            max_history: 500,
        }
    }

    fn send_to(&self, session_id: SessionId, msg: ServerMsg) {
        if let Some(session) = self.sessions.get(&session_id) {
            let line = format_for_mode(&msg, session.mode);
            let _ = session.tx.send(line);
        }
    }

    fn broadcast(&self, msg: &ServerMsg) {
        for session in self.sessions.values() {
            if session.nick.is_some() {
                let line = format_for_mode(msg, session.mode);
                let _ = session.tx.send(line);
            }
        }
    }

    fn broadcast_except(&self, sender_id: SessionId, msg: &ServerMsg) {
        for (id, session) in &self.sessions {
            if session.nick.is_some() && *id != sender_id {
                let line = format_for_mode(msg, session.mode);
                let _ = session.tx.send(line);
            }
        }
    }

    fn broadcast_presence(&self) {
        let msg = ServerMsg::Presence {
            online: self.online_count(),
            users: self.nick_list(),
        };
        self.broadcast(&msg);
    }

    fn push_history(&mut self, line: ChatLine) {
        if self.history.len() >= self.max_history {
            self.history.pop_front();
        }
        self.history.push_back(line);
    }

    fn push_dm_history(&mut self, from: &str, to: &str, line: ChatLine) {
        let key = dm_key(from, to);
        let entry = self.dm_history.entry(key).or_default();
        if entry.len() >= self.max_history {
            entry.pop_front();
        }
        entry.push_back(line);
    }

    fn get_dm_history(&self, a: &str, b: &str) -> Vec<ChatLine> {
        let key = dm_key(a, b);
        self.dm_history
            .get(&key)
            .map(|h| h.iter().cloned().collect())
            .unwrap_or_default()
    }

    fn nick_in_use(&self, nick: &str) -> bool {
        self.sessions
            .values()
            .any(|s| s.nick.as_deref() == Some(nick))
    }

    fn find_session_id_by_nick(&self, nick: &str) -> Option<SessionId> {
        self.sessions
            .iter()
            .find(|(_, s)| s.nick.as_deref() == Some(nick))
            .map(|(id, _)| *id)
    }

    fn online_count(&self) -> usize {
        self.sessions
            .values()
            .filter(|s| s.nick.is_some())
            .count()
    }

    fn nick_list(&self) -> Vec<String> {
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

fn valid_nick(nick: &str) -> bool {
    !nick.is_empty()
        && nick.len() <= 32
        && nick
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Spawn the state manager actor. Returns a handle for sending commands.
pub fn spawn_state_actor() -> StateHandle {
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(state_actor_loop(rx));
    StateHandle { tx }
}

/// The main actor loop — owns all state, processes commands sequentially.
async fn state_actor_loop(mut rx: mpsc::UnboundedReceiver<StateCmd>) {
    let mut state = ChatState::new();

    while let Some(cmd) = rx.recv().await {
        match cmd {
            StateCmd::AddSession { session } => {
                state.sessions.insert(session.id, session);
            }
            StateCmd::RemoveSession { session_id } => {
                let nick = state.sessions.remove(&session_id).and_then(|s| s.nick);
                if let Some(nick) = nick {
                    state.broadcast(&ServerMsg::System {
                        body: format!("{nick} has left the room"),
                    });
                    state.broadcast_presence();
                }
            }
            StateCmd::Identify {
                session_id,
                nick,
                mode,
                reply,
            } => {
                let result = handle_identify(&mut state, session_id, &nick, mode);
                let _ = reply.send(result);
            }
            StateCmd::Chat { session_id, body } => {
                handle_chat(&mut state, session_id, &body);
            }
            StateCmd::Dm {
                session_id,
                to,
                body,
            } => {
                handle_dm(&mut state, session_id, &to, &body);
            }
            StateCmd::GetDmHistory {
                session_id,
                partner,
            } => {
                if let Some(nick) = state
                    .sessions
                    .get(&session_id)
                    .and_then(|s| s.nick.clone())
                {
                    let history = state.get_dm_history(&nick, &partner);
                    state.send_to(
                        session_id,
                        ServerMsg::DmHistory {
                            partner,
                            messages: history,
                        },
                    );
                }
            }
            StateCmd::Who { session_id } => {
                state.send_to(
                    session_id,
                    ServerMsg::Presence {
                        online: state.online_count(),
                        users: state.nick_list(),
                    },
                );
            }
            StateCmd::SendError { session_id, body } => {
                state.send_to(session_id, ServerMsg::Error { body });
            }
        }
    }
}

fn handle_identify(
    state: &mut ChatState,
    session_id: SessionId,
    nick: &str,
    mode: ClientMode,
) -> Result<IdentifyOk, String> {
    // If session is already identified, reject with a sentinel error
    if let Some(session) = state.sessions.get(&session_id) {
        if session.nick.is_some() {
            return Err("already_identified".into());
        }
    }

    if !valid_nick(nick) {
        return Err("Invalid nick. Use 1-32 chars: a-z A-Z 0-9 _ -".into());
    }

    if state.nick_in_use(nick) {
        return Err(format!("Nick '{}' is already in use", nick));
    }

    if let Some(session) = state.sessions.get_mut(&session_id) {
        session.nick = Some(nick.to_string());
        session.mode = mode;
    }

    // Send welcome
    let online = state.online_count();
    state.send_to(
        session_id,
        ServerMsg::Welcome {
            nick: nick.to_string(),
            room: "Chat Room".to_string(),
            online,
        },
    );

    // Send history
    let history: Vec<_> = state.history.iter().cloned().collect();
    if !history.is_empty() {
        state.send_to(session_id, ServerMsg::History { messages: history });
    }

    // Announce join
    state.broadcast(&ServerMsg::System {
        body: format!("{nick} has joined the room"),
    });
    state.broadcast_presence();

    Ok(IdentifyOk {
        nick: nick.to_string(),
        room: "Chat Room".to_string(),
        online,
    })
}

fn handle_chat(state: &mut ChatState, session_id: SessionId, body: &str) {
    let body = body.trim();
    if body.is_empty() {
        return;
    }

    let nick = match state.sessions.get(&session_id).and_then(|s| s.nick.clone()) {
        Some(nick) => nick,
        None => return,
    };

    let timestamp = super::now_hms();
    state.push_history(ChatLine {
        from: nick.clone(),
        body: body.to_string(),
        timestamp: timestamp.clone(),
    });

    state.broadcast_except(
        session_id,
        &ServerMsg::Chat {
            from: nick,
            body: body.to_string(),
            timestamp,
        },
    );
}

fn handle_dm(state: &mut ChatState, session_id: SessionId, to_nick: &str, body: &str) {
    let body = body.trim();
    if body.is_empty() {
        return;
    }

    let nick = match state.sessions.get(&session_id).and_then(|s| s.nick.clone()) {
        Some(nick) => nick,
        None => return,
    };

    if !state.nick_in_use(to_nick) {
        state.send_to(
            session_id,
            ServerMsg::Error {
                body: format!("User '{}' not found or not online", to_nick),
            },
        );
        return;
    }

    let timestamp = super::now_hms();
    state.push_dm_history(
        &nick,
        to_nick,
        ChatLine {
            from: nick.clone(),
            body: body.to_string(),
            timestamp: timestamp.clone(),
        },
    );

    let dm_msg = ServerMsg::Dm {
        from: nick,
        to: to_nick.to_string(),
        body: body.to_string(),
        timestamp,
    };

    // Only send to recipient; sender adds message locally in the UI
    if let Some(to_session_id) = state.find_session_id_by_nick(to_nick) {
        state.send_to(to_session_id, dm_msg);
    }
}
