pub mod state;
pub mod ui;

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, mpsc::Sender};

use anyhow::Result;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

use crate::protocol::ClientMsg;

/// Run the TCP client in a blocking thread with its own tokio runtime.
pub fn connect_threaded(
    ip: &str,
    port: u16,
    nick: &str,
    tx: Sender<String>,
    outgoing: Arc<Mutex<VecDeque<String>>>,
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
                _ = tokio::time::sleep(tokio::time::Duration::from_millis(50)) => {
                    let mut queue = outgoing.lock().unwrap();
                    while let Some(msg) = queue.pop_front() {
                        if writer.write_all(format!("{msg}\n").as_bytes()).await.is_err() {
                            break;
                        }
                        let _ = writer.flush().await;
                    }
                }
            }
        }

        Ok(())
    })
}
