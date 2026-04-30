use super::state::{ClientMode, SessionId, StateHandle};
use crate::protocol::ClientMsg;

/// Route an incoming line from a client to the appropriate handler.
/// `identified` tracks whether this session has already registered a nick.
pub async fn handle_line(session_id: SessionId, line: &str, state: &StateHandle) {
    // We need to determine if the client is identified.
    // The actor handles identification internally — we attempt identification
    // if the line looks like a Hello or plain nick. If the actor says the session
    // is already identified, it processes it as a message instead.
    //
    // Strategy: try to parse as a known message. If it's a Hello, always send
    // Identify. Otherwise, send as a regular command. The actor will ignore
    // commands from unidentified sessions where appropriate.

    if line.starts_with('{') {
        // JSON message
        if let Ok(msg) = serde_json::from_str::<ClientMsg>(line) {
            dispatch_client_msg(session_id, msg, state).await;
        }
    } else {
        // Plain text: could be initial nick or a command/message
        handle_plain_text(session_id, line, state).await;
    }
}

/// Handle a parsed ClientMsg.
async fn dispatch_client_msg(session_id: SessionId, msg: ClientMsg, state: &StateHandle) {
    match msg {
        ClientMsg::Hello { nick } => {
            let nick = nick.trim().to_string();
            let result = state.identify(session_id, nick, ClientMode::Json).await;
            if let Err(err_msg) = result {
                // Send error back to client via a direct send through the actor
                // We'll use a simple approach: send an error chat command
                // Actually, we need to send an Error ServerMsg. Let's use a dedicated path.
                send_error(session_id, &err_msg, state);
            }
        }
        ClientMsg::Chat { body } => {
            state.chat(session_id, body);
        }
        ClientMsg::Dm { to, body } => {
            state.dm(session_id, to, body);
        }
        ClientMsg::GetDmHistory { partner } => {
            state.get_dm_history(session_id, partner);
        }
        ClientMsg::Who => {
            state.who(session_id);
        }
        ClientMsg::Quit => {
            state.remove_session(session_id);
        }
    }
}

/// Handle plain-text input (telnet/nc clients).
async fn handle_plain_text(session_id: SessionId, line: &str, state: &StateHandle) {
    // Plain text commands for identified users
    if let Some(rest) = line.strip_prefix("/dm ") {
        let parts: Vec<&str> = rest.splitn(2, ' ').collect();
        if parts.len() == 2 {
            state.dm(session_id, parts[0].trim().to_string(), parts[1].to_string());
        } else {
            send_error(session_id, "Usage: /dm <username> <message>", state);
        }
    } else if let Some(rest) = line.strip_prefix("/dmhistory ") {
        let partner = rest.trim().to_string();
        state.get_dm_history(session_id, partner);
    } else if line == "/who" {
        state.who(session_id);
    } else if line == "/quit" {
        state.remove_session(session_id);
    } else {
        // Could be initial nick identification or a chat message.
        // Try identification first — if it fails because the session is
        // already identified, treat it as a chat message.
        let nick = line.trim().to_string();
        let result = state.identify(session_id, nick.clone(), ClientMode::Plain).await;
        match result {
            Ok(_) => { /* Successfully identified */ }
            Err(ref e) if e == "already_identified" => {
                // Session already has a nick, this is a chat message
                state.chat(session_id, nick);
            }
            Err(err_msg) => {
                send_error(session_id, &err_msg, state);
            }
        }
    }
}

/// Send an error message to a session via the state actor's error path.
fn send_error(session_id: SessionId, msg: &str, state: &StateHandle) {
    // We use a special DM-like approach: send an Error ServerMsg.
    // Since the actor owns the session tx channels, we need a way to send errors.
    // We'll add an inline error command. For now, we can use the state handle's
    // internal tx to send a special command. But to keep things simple and avoid
    // adding another command variant just for errors, we'll use a lightweight approach:
    // The Identify command already returns errors via oneshot. For other errors,
    // we add a SendError command.
    let _ = state.tx.send(super::state::StateCmd::SendError {
        session_id,
        body: msg.to_string(),
    });
}
