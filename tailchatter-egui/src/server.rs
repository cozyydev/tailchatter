use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, mpsc};

type SessionId = u64;

const DEFAULT_ROOM: &str = "Chat Room";
const MAX_HISTORY: usize = 50;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChatLine {
    from: String,
    body: String,
    timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMsg {
    Hello { nick: String },
    Chat { body: String },
    Who,
    Quit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerMsg {
    Welcome { nick: String, room: String, online: usize },
    System { body: String },
    Chat { from: String, body: String, timestamp: String },
    Presence { online: usize, users: Vec<String> },
    History { messages: Vec<ChatLine> },
    Error { body: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClientMode {
    Plain,
    Json,
}

#[derive(Debug)]
struct Session {
    nick: Option<String>,
    mode: Option<ClientMode>,
    tx: mpsc::UnboundedSender<String>,
}

#[derive(Debug)]
struct ChatState {
    sessions: HashMap<SessionId, Session>,
    history: VecDeque<ChatLine>,
}

impl ChatState {
    fn new() -> Self {
        Self {
            sessions: HashMap::new(),
            history: VecDeque::new(),
        }
    }

    fn online_count(&self) -> usize {
        self.sessions
            .values()
            .filter(|session| session.nick.is_some())
            .count()
    }

    fn nick_list(&self) -> Vec<String> {
        let mut users: Vec<String> = self
            .sessions
            .values()
            .filter_map(|session| session.nick.clone())
            .collect();
        users.sort();
        users
    }

    fn nick_in_use(&self, nick: &str) -> bool {
        self.sessions
            .values()
            .filter_map(|session| session.nick.as_deref())
            .any(|existing| existing.eq_ignore_ascii_case(nick))
    }

    fn push_history(&mut self, line: ChatLine) {
        self.history.push_back(line);
        while self.history.len() > MAX_HISTORY {
            self.history.pop_front();
        }
    }

    fn send_raw(&self, session_id: SessionId, line: impl Into<String>) {
        if let Some(session) = self.sessions.get(&session_id) {
            let _ = session.tx.send(line.into());
        }
    }

    fn send_to(&self, session_id: SessionId, msg: ServerMsg) {
        if let Some(session) = self.sessions.get(&session_id) {
            let mode = session.mode.unwrap_or(ClientMode::Plain);
            let formatted = format_for_mode(&msg, mode);
            if !formatted.is_empty() {
                let _ = session.tx.send(formatted);
            }
        }
    }

    fn broadcast(&self, msg: ServerMsg) {
        for session in self.sessions.values() {
            if session.nick.is_some() {
                let mode = session.mode.unwrap_or(ClientMode::Plain);
                let formatted = format_for_mode(&msg, mode);
                if !formatted.is_empty() {
                    let _ = session.tx.send(formatted);
                }
            }
        }
    }

    fn broadcast_presence(&self) {
        let msg = ServerMsg::Presence {
            online: self.online_count(),
            users: self.nick_list(),
        };
        self.broadcast(msg);
    }

    fn broadcast_chat_from(&self, _sender_id: SessionId, msg: ServerMsg) {
        self.broadcast(msg);
    }
}

fn format_for_mode(msg: &ServerMsg, mode: ClientMode) -> String {
    match mode {
        ClientMode::Json => serde_json::to_string(msg).unwrap_or_else(|_| {
            "{\"type\":\"error\",\"body\":\"serialization error\"}".to_string()
        }),
        ClientMode::Plain => format_plain(msg),
    }
}

fn format_plain(msg: &ServerMsg) -> String {
    match msg {
        ServerMsg::System { body } => body.clone(),
        ServerMsg::Chat {
            from,
            body,
            timestamp: _,
        } => format!("{from}: {body}"),
        ServerMsg::History { messages } => {
            if messages.is_empty() {
                String::new()
            } else {
                messages
                    .iter()
                    .map(|m| format!("{}: {}", m.from, m.body))
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        }
        ServerMsg::Welcome { .. } => String::new(),
        ServerMsg::Presence { online, users } => {
            format!("Online ({online}): {}", users.join(", "))
        }
        ServerMsg::Error { body } => format!("Error: {body}"),
    }
}

fn now_hms() -> String {
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

fn valid_nick(nick: &str) -> bool {
    let nick = nick.trim();
    if nick.len() < 2 || nick.len() > 24 {
        return false;
    }
    nick.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

async fn handle_client(stream: TcpStream, session_id: SessionId, state: Arc<Mutex<ChatState>>) -> Result<()> {
    let (reader, writer) = stream.into_split();
    let reader = BufReader::new(reader);
    let mut lines = reader.lines();
    let mut writer = BufWriter::new(writer);

    let (tx, mut rx) = mpsc::unbounded_channel::<String>();

    {
        let mut state = state.lock().await;
        state.sessions.insert(
            session_id,
            Session {
                nick: None,
                mode: None,
                tx,
            },
        );
    }

    let writer_task = tokio::spawn(async move {
        while let Some(line) = rx.recv().await {
            if writer.write_all(line.as_bytes()).await.is_err() {
                break;
            }
            if writer.write_all(b"\n").await.is_err() {
                break;
            }
            if writer.flush().await.is_err() {
                break;
            }
        }
    });

    while let Some(line) = lines.next_line().await? {
        handle_line(session_id, &line, &state).await;
    }

    disconnect_session(session_id, &state).await;
    writer_task.abort();

    Ok(())
}

async fn handle_line(session_id: SessionId, line: &str, state: &Arc<Mutex<ChatState>>) {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return;
    }

    let identified = {
        let state = state.lock().await;
        state.sessions.get(&session_id).and_then(|s| s.nick.as_ref()).is_some()
    };

    if !identified {
        handle_identification(session_id, trimmed, state).await;
    } else {
        handle_chat_input(session_id, trimmed, state).await;
    }
}

async fn handle_identification(session_id: SessionId, line: &str, state: &Arc<Mutex<ChatState>>) {
    let (mode, nick) = if line.starts_with('{') {
        match serde_json::from_str::<ClientMsg>(line) {
            Ok(ClientMsg::Hello { nick }) => (ClientMode::Json, nick.trim().to_string()),
            Ok(_) => {
                let state = state.lock().await;
                state.send_to(
                    session_id,
                    ServerMsg::Error { body: "First message must be your handle".to_string() },
                );
                return;
            }
            Err(_) => {
                let state = state.lock().await;
                state.send_to(
                    session_id,
                    ServerMsg::Error { body: "Invalid JSON hello message".to_string() },
                );
                return;
            }
        }
    } else {
        (ClientMode::Plain, line.to_string())
    };

    let mut state = state.lock().await;

    if state.sessions.get(&session_id).and_then(|s| s.nick.as_ref()).is_some() {
        state.send_to(
            session_id,
            ServerMsg::Error { body: "You already set your handle".to_string() },
        );
        return;
    }

    if !valid_nick(&nick) {
        state.send_to(
            session_id,
            ServerMsg::Error { body: "Handle must be 2-24 chars: letters, numbers, _ or -".to_string() },
        );
        return;
    }

    if state.nick_in_use(&nick) {
        state.send_to(
            session_id,
            ServerMsg::Error { body: "That handle is already in use".to_string() },
        );
        return;
    }

    if let Some(session) = state.sessions.get_mut(&session_id) {
        session.nick = Some(nick.clone());
        session.mode = Some(mode);
    }

    state.send_to(
        session_id,
        ServerMsg::Welcome {
            nick: nick.clone(),
            room: DEFAULT_ROOM.to_string(),
            online: state.online_count(),
        },
    );

    state.send_to(
        session_id,
        ServerMsg::History { messages: state.history.iter().cloned().collect() },
    );

    state.broadcast(ServerMsg::System {
        body: format!("{nick} has joined the room"),
    });

    state.broadcast_presence();
}

async fn handle_chat_input(session_id: SessionId, line: &str, state: &Arc<Mutex<ChatState>>) {
    if line.starts_with('{') {
        if let Ok(msg) = serde_json::from_str::<ClientMsg>(line) {
            match msg {
                ClientMsg::Chat { body } => {
                    send_chat(session_id, body.trim(), state).await;
                }
                ClientMsg::Who => {
                    let state = state.lock().await;
                    state.send_to(
                        session_id,
                        ServerMsg::Presence {
                            online: state.online_count(),
                            users: state.nick_list(),
                        },
                    );
                }
                ClientMsg::Quit => {
                    disconnect_session(session_id, state).await;
                }
                ClientMsg::Hello { .. } => {
                    let state = state.lock().await;
                    state.send_to(
                        session_id,
                        ServerMsg::Error { body: "You already set your handle".to_string() },
                    );
                }
            }
            return;
        }
    }

    match line {
        "/who" => {
            let state = state.lock().await;
            state.send_to(
                session_id,
                ServerMsg::Presence {
                    online: state.online_count(),
                    users: state.nick_list(),
                },
            );
        }
        "/quit" => {
            disconnect_session(session_id, state).await;
        }
        _ => {
            send_chat(session_id, line, state).await;
        }
    }
}

async fn send_chat(session_id: SessionId, body: &str, state: &Arc<Mutex<ChatState>>) {
    let body = body.trim();
    if body.is_empty() {
        return;
    }

    let mut state = state.lock().await;

    let nick = match state.sessions.get(&session_id).and_then(|s| s.nick.clone()) {
        Some(nick) => nick,
        None => {
            state.send_to(
                session_id,
                ServerMsg::Error { body: "Set your nickname first".to_string() },
            );
            return;
        }
    };

    let timestamp = now_hms();
    let entry = ChatLine {
        from: nick.clone(),
        body: body.to_string(),
        timestamp: timestamp.clone(),
    };

    state.push_history(entry);

    state.broadcast_chat_from(
        session_id,
        ServerMsg::Chat {
            from: nick,
            body: body.to_string(),
            timestamp,
        },
    );
}

async fn disconnect_session(session_id: SessionId, state: &Arc<Mutex<ChatState>>) {
    let mut state = state.lock().await;

    let nick = state.sessions.remove(&session_id).and_then(|session| session.nick);

    if let Some(nick) = nick {
        state.broadcast(ServerMsg::System {
            body: format!("{nick} has left the room"),
        });
        state.broadcast_presence();
    }
}

pub async fn start_server(port: u16) -> anyhow::Result<()> {
    let addr = format!("0.0.0.0:{port}");
    let listener = TcpListener::bind(&addr).await?;
    let state = Arc::new(Mutex::new(ChatState::new()));
    let state_clone = Arc::clone(&state);

    println!("TailChatter server listening on {addr}");

    let mut next_session_id: SessionId = 1;

    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, _addr)) => {
                    let state = Arc::clone(&state_clone);
                    let session_id = next_session_id;
                    next_session_id += 1;

                    tokio::spawn(async move {
                        if let Err(err) = handle_client(stream, session_id, state).await {
                            eprintln!("session {session_id} error: {err}");
                        }
                    });
                }
                Err(err) => {
                    eprintln!("accept error: {}", err);
                }
            }
        }
    });

    Ok(())
}