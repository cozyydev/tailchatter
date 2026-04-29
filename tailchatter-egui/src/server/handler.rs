use std::sync::Arc;
use tokio::sync::Mutex;

use super::now_hms;
use super::state::{ChatState, ClientMode, SessionId};
use crate::protocol::{ChatLine, ClientMsg, ServerMsg};

/// Route an incoming line from a client to the appropriate handler.
pub async fn handle_line(session_id: SessionId, line: &str, state: &Arc<Mutex<ChatState>>) {
    let is_unidentified = {
        let state = state.lock().await;
        state
            .sessions
            .get(&session_id)
            .and_then(|s| s.nick.as_ref())
            .is_none()
    };

    if is_unidentified {
        handle_identification(session_id, line, state).await;
    } else {
        handle_message(session_id, line, state).await;
    }
}

/// Handle the first message from a client (nick identification).
async fn handle_identification(session_id: SessionId, line: &str, state: &Arc<Mutex<ChatState>>) {
    // JSON hello (from GUI client)
    if line.starts_with('{') {
        if let Ok(ClientMsg::Hello { nick }) = serde_json::from_str::<ClientMsg>(line) {
            let nick = nick.trim().to_string();
            if let Err(msg) =
                validate_and_register(session_id, &nick, ClientMode::Json, state).await
            {
                let state = state.lock().await;
                state.send_to(session_id, ServerMsg::Error { body: msg });
            }
        } else {
            let state = state.lock().await;
            state.send_to(
                session_id,
                ServerMsg::Error {
                    body: "Expected hello message. Send: {\"type\":\"hello\",\"nick\":\"...\"}"
                        .into(),
                },
            );
        }
        return;
    }

    // Plain text handle (telnet/nc clients)
    let nick = line.trim().to_string();
    if let Err(msg) = validate_and_register(session_id, &nick, ClientMode::Plain, state).await {
        let state = state.lock().await;
        state.send_to(session_id, ServerMsg::Error { body: msg });
    }
}

/// Validate handle and register session. Returns Err(message) on failure.
async fn validate_and_register(
    session_id: SessionId,
    nick: &str,
    mode: ClientMode,
    state: &Arc<Mutex<ChatState>>,
) -> Result<(), String> {
    if !valid_nick(nick) {
        return Err("Invalid nick. Use 1-32 chars: a-z A-Z 0-9 _ -".into());
    }

    let mut state = state.lock().await;

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

    Ok(())
}

/// Handle a message from an identified client.
async fn handle_message(session_id: SessionId, line: &str, state: &Arc<Mutex<ChatState>>) {
    // JSON messages from GUI client
    if line.starts_with('{') {
        if let Ok(msg) = serde_json::from_str::<ClientMsg>(line) {
            dispatch_client_msg(session_id, msg, state).await;
        }
        return;
    }

    // Plain text commands
    if line.starts_with("/dm ") {
        let parts: Vec<&str> = line[4..].splitn(2, ' ').collect();
        if parts.len() == 2 {
            send_dm(session_id, parts[0].trim(), parts[1], state).await;
        } else {
            let state = state.lock().await;
            state.send_to(
                session_id,
                ServerMsg::Error {
                    body: "Usage: /dm <username> <message>".into(),
                },
            );
        }
    } else if line.starts_with("/dmhistory ") {
        let partner = line[11..].trim();
        let state = state.lock().await;
        if let Some(nick) = state.sessions.get(&session_id).and_then(|s| s.nick.clone()) {
            let history = state.get_dm_history(&nick, partner);
            state.send_to(
                session_id,
                ServerMsg::DmHistory {
                    partner: partner.to_string(),
                    messages: history,
                },
            );
        }
    } else if line == "/who" {
        let state = state.lock().await;
        state.send_to(
            session_id,
            ServerMsg::Presence {
                online: state.online_count(),
                users: state.nick_list(),
            },
        );
    } else if line == "/quit" {
        disconnect_session(session_id, state).await;
    } else {
        send_chat(session_id, line, state).await;
    }
}

/// Dispatch a parsed ClientMsg to the appropriate handler.
async fn dispatch_client_msg(session_id: SessionId, msg: ClientMsg, state: &Arc<Mutex<ChatState>>) {
    match msg {
        ClientMsg::Chat { body } => send_chat(session_id, &body, state).await,
        ClientMsg::Dm { to, body } => send_dm(session_id, &to, &body, state).await,
        ClientMsg::GetDmHistory { partner } => {
            let state = state.lock().await;
            if let Some(nick) = state.sessions.get(&session_id).and_then(|s| s.nick.clone()) {
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
        ClientMsg::Quit => disconnect_session(session_id, state).await,
        ClientMsg::Hello { .. } => {} // Ignore Hello after identification
    }
}

/// Broadcast a chat message to the room (excluding sender).
async fn send_chat(session_id: SessionId, body: &str, state: &Arc<Mutex<ChatState>>) {
    let body = body.trim();
    if body.is_empty() {
        return;
    }

    let mut state = state.lock().await;

    let nick = match state.sessions.get(&session_id).and_then(|s| s.nick.clone()) {
        Some(nick) => nick,
        None => return,
    };

    let timestamp = now_hms();
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

/// Send a direct message to a specific user.
async fn send_dm(session_id: SessionId, to_nick: &str, body: &str, state: &Arc<Mutex<ChatState>>) {
    let body = body.trim();
    if body.is_empty() {
        return;
    }

    let mut state = state.lock().await;

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

    let timestamp = now_hms();
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

/// Remove a session and announce departure.
pub async fn disconnect_session(session_id: SessionId, state: &Arc<Mutex<ChatState>>) {
    let mut state = state.lock().await;
    let nick = state.sessions.remove(&session_id).and_then(|s| s.nick);

    if let Some(nick) = nick {
        state.broadcast(&ServerMsg::System {
            body: format!("{nick} has left the room"),
        });
        state.broadcast_presence();
    }
}

fn valid_nick(nick: &str) -> bool {
    !nick.is_empty()
        && nick.len() <= 32
        && nick
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}
