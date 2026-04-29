use serde::{Deserialize, Serialize};

/// A single chat message with sender, body, and timestamp.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatLine {
    pub from: String,
    pub body: String,
    pub timestamp: String,
}

/// Messages sent from client to server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    Hello { nick: String },
    Chat { body: String },
    Dm { to: String, body: String },
    GetDmHistory { partner: String },
    Who,
    Quit,
}

/// Messages sent from server to client.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    Welcome {
        nick: String,
        room: String,
        online: usize,
    },
    System {
        body: String,
    },
    Chat {
        from: String,
        body: String,
        timestamp: String,
    },
    Dm {
        from: String,
        to: String,
        body: String,
        timestamp: String,
    },
    Presence {
        online: usize,
        users: Vec<String>,
    },
    History {
        messages: Vec<ChatLine>,
    },
    DmHistory {
        partner: String,
        messages: Vec<ChatLine>,
    },
    Error {
        body: String,
    },
}
