pub mod handler;
pub mod state;

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::mpsc;

use crate::protocol::ServerMsg;
use state::{ClientMode, Session, SessionId, StateHandle, spawn_state_actor};

static NEXT_SESSION_ID: AtomicU32 = AtomicU32::new(1);

fn new_session_id() -> SessionId {
    NEXT_SESSION_ID.fetch_add(1, Ordering::SeqCst)
}

/// Start the TCP chat server on the given port (non-blocking, spawns thread).
pub fn start(port: u16) -> Result<()> {
    let addr = format!("0.0.0.0:{port}");

    // Test bind synchronously to fail fast if port is in use
    let test = std::net::TcpListener::bind(&addr);
    if let Err(e) = test {
        return Err(e.into());
    }
    drop(test);

    println!("TailChatter server listening on {addr}");

    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();

        rt.block_on(async {
            let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();

            // Spawn the state actor — no mutex needed
            let state_handle = spawn_state_actor();

            loop {
                match listener.accept().await {
                    Ok((stream, _)) => {
                        let handle = state_handle.clone();
                        let session_id = new_session_id();
                        tokio::spawn(handle_client(stream, session_id, handle));
                    }
                    Err(err) => {
                        eprintln!("accept error: {err}");
                    }
                }
            }
        });
    });

    Ok(())
}

/// Handle a single client connection lifecycle.
async fn handle_client(stream: TcpStream, session_id: SessionId, state: StateHandle) {
    let (reader, writer) = tokio::io::split(stream);
    let mut reader = BufReader::new(reader);

    let (tx, mut rx) = mpsc::unbounded_channel::<String>();

    // Register this session with the actor
    state.add_session(Session {
        id: session_id,
        nick: None,
        mode: ClientMode::Plain,
        tx,
    });

    // Writer task: forward channel messages to TCP socket
    let writer_task = tokio::spawn(async move {
        let mut writer = writer;
        while let Some(msg) = rx.recv().await {
            if writer.write_all(msg.as_bytes()).await.is_err() {
                break;
            }
            let _ = writer.write_all(b"\n").await;
        }
    });

    // Reader loop: process incoming lines
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    handler::handle_line(session_id, trimmed, &state).await;
                }
            }
        }
    }

    state.remove_session(session_id);
    writer_task.abort();
}

/// Format a ServerMsg for the given client mode.
pub fn format_for_mode(msg: &ServerMsg, mode: ClientMode) -> String {
    match mode {
        ClientMode::Json => serde_json::to_string(msg).unwrap_or_default(),
        ClientMode::Plain => format_plain(msg),
    }
}

fn format_plain(msg: &ServerMsg) -> String {
    match msg {
        ServerMsg::Chat { from, body, timestamp } => format!("[{timestamp}] {from}: {body}"),
        ServerMsg::Dm { from, to, body, timestamp } => {
            format!("[{timestamp}] DM from {from} to {to}: {body}")
        }
        ServerMsg::System { body } => format!("*** {body}"),
        ServerMsg::Presence { online, users } => {
            format!("*** Online: {online} users: {}", users.join(", "))
        }
        ServerMsg::History { messages } => messages
            .iter()
            .map(|m| format!("[{}] {}: {}", m.timestamp, m.from, m.body))
            .collect::<Vec<_>>()
            .join("\n"),
        ServerMsg::DmHistory { partner, messages } => {
            let lines: Vec<_> = messages
                .iter()
                .map(|m| format!("[{}] {}: {}", m.timestamp, m.from, m.body))
                .collect();
            format!("*** DM history with {partner}:\n{}", lines.join("\n"))
        }
        ServerMsg::Welcome { nick, room, online } => {
            format!("*** Welcome to {room}, {nick}! ({online} online)")
        }
        ServerMsg::Error { body } => format!("*** Error: {body}"),
    }
}

/// Get current time as HH:MM:SS.
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
