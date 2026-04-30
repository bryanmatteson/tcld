use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::{Mutex, mpsc};

use super::protocol::{AgentCommand, AgentEvent};

/// Manages the TCP connection to the Tcl debug agent.
pub struct Bridge {
    writer: Arc<Mutex<Option<tokio::io::WriteHalf<TcpStream>>>>,
    event_rx: mpsc::Receiver<AgentEvent>,
}

impl Bridge {
    /// Connect to the agent's TCP server. Returns the bridge once connected.
    pub async fn connect(port: u16, timeout_secs: u64) -> Result<Self, String> {
        let addr = format!("127.0.0.1:{port}");

        let stream = tokio::time::timeout(
            std::time::Duration::from_secs(timeout_secs),
            TcpStream::connect(&addr),
        )
        .await
        .map_err(|_| format!("timeout connecting to agent on port {port}"))?
        .map_err(|e| format!("failed to connect to agent: {e}"))?;

        let (reader, writer) = tokio::io::split(stream);
        let writer = Arc::new(Mutex::new(Some(writer)));

        let (event_tx, event_rx) = mpsc::channel(256);

        // Spawn reader task
        let writer_clone = writer.clone();
        tokio::spawn(async move {
            let mut buf_reader = BufReader::new(reader);
            let mut line = String::new();

            loop {
                line.clear();
                match buf_reader.read_line(&mut line).await {
                    Ok(0) => {
                        // EOF — agent disconnected
                        let _ = event_tx
                            .send(AgentEvent::Exited {
                                code: -1,
                                reason: Some("agent disconnected".into()),
                            })
                            .await;
                        break;
                    }
                    Ok(_) => {
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        match AgentEvent::parse_tolerant(trimmed) {
                            Ok(event) => {
                                if event_tx.send(event).await.is_err() {
                                    break;
                                }
                            }
                            Err(e) => {
                                eprintln!(
                                    "tcld-bridge: failed to parse agent event: {e}\n  line: {trimmed}"
                                );
                            }
                        }
                    }
                    Err(e) => {
                        let _ = event_tx
                            .send(AgentEvent::Exited {
                                code: -1,
                                reason: Some(format!("socket read error: {e}")),
                            })
                            .await;
                        break;
                    }
                }
            }

            // Clean up writer
            let mut w = writer_clone.lock().await;
            *w = None;
        });

        Ok(Bridge { writer, event_rx })
    }

    /// Send a command to the agent.
    pub async fn send(&self, cmd: &AgentCommand) -> Result<(), String> {
        let json = serde_json::to_string(cmd).map_err(|e| format!("serialize error: {e}"))?;

        let mut w = self.writer.lock().await;
        let writer = w.as_mut().ok_or("bridge disconnected")?;

        writer
            .write_all(json.as_bytes())
            .await
            .map_err(|e| format!("socket write error: {e}"))?;
        writer
            .write_all(b"\n")
            .await
            .map_err(|e| format!("socket write error: {e}"))?;
        writer
            .flush()
            .await
            .map_err(|e| format!("socket flush error: {e}"))?;

        Ok(())
    }

    /// Take the event receiver out of the bridge. The event loop owns it directly
    /// so it doesn't need to hold a lock on the session to receive events.
    pub fn take_event_rx(&mut self) -> mpsc::Receiver<AgentEvent> {
        let (_, empty_rx) = mpsc::channel(1);
        std::mem::replace(&mut self.event_rx, empty_rx)
    }

    /// Disconnect the bridge.
    pub async fn disconnect(&self) {
        let mut w = self.writer.lock().await;
        if let Some(mut writer) = w.take() {
            let _ = writer.shutdown().await;
        }
    }
}
