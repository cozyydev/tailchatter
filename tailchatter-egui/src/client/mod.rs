pub mod state;
pub mod ui;

use std::sync::mpsc::Sender;

use anyhow::Result;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::protocol::ClientMsg;

/// Run the TCP client in a blocking thread with its own tokio runtime.
/// Uses a proper async channel for outgoing messages (no polling).
pub fn connect_threaded(
    ip: &str,
    port: u16,
    nick: &str,
    tx: Sender<String>,
    mut outgoing_rx: UnboundedReceiver<String>,
) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    let nick = nick.to_string();
    let ip = ip.to_string();

    runtime.block_on(async {
        let addr = format!("{ip}:{port}");
        let stream = TcpStream::connect(&addr).await?;

        let (reader, mut writer) = stream.into_split();
        let mut lines = BufReader::new(reader).lines();

        // Send Hello as first message
        let hello = serde_json::to_string(&ClientMsg::Hello { nick }).unwrap();
        writer.write_all(format!("{hello}\n").as_bytes()).await?;
        writer.flush().await?;

        loop {
            tokio::select! {
                result = lines.next_line() => {
                    match result {
                        Ok(Some(line)) => {
                            let _ = tx.send(line);
                        }
                        Ok(None) | Err(_) => break,
                    }
                }
                Some(msg) = outgoing_rx.recv() => {
                    if writer.write_all(format!("{msg}\n").as_bytes()).await.is_err() {
                        break;
                    }
                    // Drain any additional queued messages without blocking
                    while let Ok(msg) = outgoing_rx.try_recv() {
                        if writer.write_all(format!("{msg}\n").as_bytes()).await.is_err() {
                            break;
                        }
                    }
                    let _ = writer.flush().await;
                }
            }
        }

        Ok(())
    })
}
