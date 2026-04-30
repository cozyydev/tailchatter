use std::collections::HashMap;
use std::sync::mpsc::Receiver;
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::sync::mpsc::UnboundedSender;

use crate::protocol::{ClientMsg, ServerMsg};

pub const DEFAULT_PORT: u16 = 42069;

/// Identifies which conversation is active.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Conversation {
    Group,
    Dm(String),
}

/// Application mode (login screen vs chat screen).
#[derive(PartialEq, Clone, Copy)]
pub enum AppMode {
    Login,
    Chat,
}

/// Which tab is active on the login screen.
#[derive(PartialEq, Clone, Copy, Default)]
pub enum LoginTab {
    #[default]
    Connect,
    Server,
}

/// Core application state.
pub struct ChatApp {
    pub mode: AppMode,
    pub login_tab: LoginTab,
    pub nick: String,
    pub server_ip: String,
    pub server_port: u16,
    pub local_server_port: u16,
    pub server_started: bool,
    pub online_users: Vec<String>,
    pub room_name: String,
    pub input_message: String,
    pub error_message: String,
    pub msg_receiver: Option<Receiver<String>>,
    pub outgoing_tx: Option<UnboundedSender<String>>,
    pub was_logged_out: bool,
    pub active_conversation: Conversation,
    pub dm_conversations: Vec<String>,
    pub conversation_messages: HashMap<Conversation, Vec<(String, String, String)>>,
    pub unread_dms: HashMap<String, usize>,
}

impl Default for ChatApp {
    fn default() -> Self {
        Self {
            mode: AppMode::Login,
            login_tab: LoginTab::default(),
            nick: String::new(),
            server_ip: String::new(),
            server_port: DEFAULT_PORT,
            local_server_port: DEFAULT_PORT,
            server_started: false,
            online_users: Vec::new(),
            room_name: "Chat Room".into(),
            input_message: String::new(),
            error_message: String::new(),
            msg_receiver: None,
            outgoing_tx: None,
            was_logged_out: false,
            active_conversation: Conversation::Group,
            dm_conversations: Vec::new(),
            conversation_messages: HashMap::new(),
            unread_dms: HashMap::new(),
        }
    }
}

impl ChatApp {
    /// Process an incoming server message and update state.
    pub fn handle_server_msg(&mut self, msg: ServerMsg) {
        match msg {
            ServerMsg::Welcome { nick, room, .. } => {
                self.room_name = room;
                self.push_group_msg("System", &format!("Welcome, {nick}!"));
            }
            ServerMsg::Chat { from, body, timestamp } => {
                let msgs = self.conversation_messages.entry(Conversation::Group).or_default();
                msgs.push((from, body, timestamp));
            }
            ServerMsg::Dm { from, to, body, timestamp } => {
                let partner = if from == self.nick { to } else { from.clone() };
                let conv = Conversation::Dm(partner.clone());
                let msgs = self.conversation_messages.entry(conv.clone()).or_default();
                msgs.push((from, body, timestamp));

                if !self.dm_conversations.contains(&partner) {
                    self.dm_conversations.push(partner.clone());
                }

                if self.active_conversation != conv {
                    *self.unread_dms.entry(partner.clone()).or_insert(0) += 1;
                }

                // Move partner to front of DM list (most recent)
                if let Some(pos) = self.dm_conversations.iter().position(|u| u == &partner) {
                    self.dm_conversations.remove(pos);
                    self.dm_conversations.insert(0, partner);
                }
            }
            ServerMsg::DmHistory { partner, messages } => {
                let conv = Conversation::Dm(partner);
                let msgs: Vec<_> = messages
                    .into_iter()
                    .map(|m| (m.from, m.body, m.timestamp))
                    .collect();
                self.conversation_messages.insert(conv, msgs);
            }
            ServerMsg::System { body } => {
                self.push_group_msg("System", &body);
            }
            ServerMsg::Presence { users, .. } => {
                self.online_users = users;
            }
            ServerMsg::History { messages } => {
                let msgs: Vec<_> = messages
                    .into_iter()
                    .map(|m| (m.from, m.body, m.timestamp))
                    .collect();
                self.conversation_messages.insert(Conversation::Group, msgs);
            }
            ServerMsg::Error { body } => {
                self.push_group_msg("Error", &body);
            }
        }
    }

    /// Fallback handler for plain text messages from server.
    pub fn handle_plain_text(&mut self, msg: &str) {
        if msg.starts_with("Online") {
            if let Some(users_part) = msg.split_once(':').map(|x| x.1) {
                self.online_users = users_part
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
        } else if msg.to_lowercase().contains("enter your handle") {
            // Ignore server prompt
        } else if msg.contains(": ") {
            let parts: Vec<&str> = msg.splitn(2, ": ").collect();
            if parts.len() == 2 {
                self.push_group_msg(parts[0], parts[1]);
            }
        }
    }

    /// Send the current input message to the server.
    pub fn send_message(&mut self) {
        if self.input_message.is_empty() {
            return;
        }

        let msg_json = match &self.active_conversation {
            Conversation::Group => {
                serde_json::to_string(&ClientMsg::Chat {
                    body: self.input_message.clone(),
                })
                .unwrap()
            }
            Conversation::Dm(partner) => {
                serde_json::to_string(&ClientMsg::Dm {
                    to: partner.clone(),
                    body: self.input_message.clone(),
                })
                .unwrap()
            }
        };

        if let Some(ref tx) = self.outgoing_tx {
            let _ = tx.send(msg_json);
        }

        // Add message locally for immediate display
        let from = self.nick.clone();
        let body = self.input_message.clone();
        let timestamp = now_hms();
        let msgs = self
            .conversation_messages
            .entry(self.active_conversation.clone())
            .or_default();
        msgs.push((from, body, timestamp));

        // Ensure DM partner is in conversations list
        if let Conversation::Dm(ref partner) = self.active_conversation {
            if !self.dm_conversations.contains(partner) {
                self.dm_conversations.push(partner.clone());
            }
        }

        self.input_message.clear();
    }

    /// Disconnect and return to login screen.
    pub fn logout(&mut self) {
        if let Some(ref tx) = self.outgoing_tx {
            let _ = tx.send(serde_json::to_string(&ClientMsg::Quit).unwrap());
        }
        self.conversation_messages.clear();
        self.online_users.clear();
        self.input_message.clear();
        self.dm_conversations.clear();
        self.unread_dms.clear();
        self.active_conversation = Conversation::Group;
        self.mode = AppMode::Login;
    }

    fn push_group_msg(&mut self, from: &str, body: &str) {
        let msgs = self.conversation_messages.entry(Conversation::Group).or_default();
        msgs.push((from.to_string(), body.to_string(), now_hms()));
    }
}

pub fn now_hms() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let secs_in_day = secs % 86_400;
    let hour = secs_in_day / 3_600;
    let minute = (secs_in_day % 3_600) / 60;
    let second = secs_in_day % 60;
    format!("{hour:02}:{minute:02}:{second:02}")
}
