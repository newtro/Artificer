//! Tokio WebSocket listener for authoritative servers.
//!
//! One accept loop; per-connection reader/writer tasks; a single ordered
//! event stream (`Connected` / `Message` / `Disconnected`) consumed by the
//! game's simulation loop. Sends go through per-connection unbounded
//! channels so a slow client never blocks the simulation.

use futures_util::{SinkExt, StreamExt};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio_tungstenite::tungstenite::Message;

pub type ConnId = u64;

#[derive(Debug)]
pub enum ServerEvent {
    Connected(ConnId, SocketAddr),
    Message(ConnId, Vec<u8>),
    Disconnected(ConnId),
}

type Senders = Arc<Mutex<HashMap<ConnId, UnboundedSender<Message>>>>;

pub struct WsServer {
    events: UnboundedReceiver<ServerEvent>,
    senders: Senders,
    local_addr: SocketAddr,
}

impl WsServer {
    /// Bind and start accepting. Must be called within a tokio runtime.
    pub async fn bind(addr: &str) -> std::io::Result<WsServer> {
        let listener = TcpListener::bind(addr).await?;
        let local_addr = listener.local_addr()?;
        let (event_tx, events) = unbounded_channel();
        let senders: Senders = Arc::new(Mutex::new(HashMap::new()));

        let accept_senders = senders.clone();
        tokio::spawn(async move {
            let next_id = AtomicU64::new(1);
            loop {
                match listener.accept().await {
                    Ok((stream, peer)) => {
                        let id = next_id.fetch_add(1, Ordering::Relaxed);
                        let tx = event_tx.clone();
                        let senders = accept_senders.clone();
                        tokio::spawn(handle_connection(stream, peer, id, tx, senders));
                    }
                    Err(e) => {
                        log::error!("accept failed: {e}");
                        break;
                    }
                }
            }
        });

        Ok(WsServer {
            events,
            senders,
            local_addr,
        })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Await the next connection event.
    pub async fn next_event(&mut self) -> Option<ServerEvent> {
        self.events.recv().await
    }

    /// Drain without waiting (call from a tick loop).
    pub fn try_events(&mut self) -> Vec<ServerEvent> {
        let mut out = Vec::new();
        while let Ok(ev) = self.events.try_recv() {
            out.push(ev);
        }
        out
    }

    /// Queue a binary message to one connection. False if it is gone.
    pub fn send(&self, id: ConnId, bytes: Vec<u8>) -> bool {
        let senders = self.senders.lock().unwrap();
        match senders.get(&id) {
            Some(tx) => tx.send(Message::binary(bytes)).is_ok(),
            None => false,
        }
    }

    /// Close a connection (writer task ends; reader observes the close).
    pub fn kick(&self, id: ConnId) {
        let mut senders = self.senders.lock().unwrap();
        if let Some(tx) = senders.remove(&id) {
            let _ = tx.send(Message::Close(None));
        }
    }

    pub fn connection_count(&self) -> usize {
        self.senders.lock().unwrap().len()
    }
}

async fn handle_connection(
    stream: TcpStream,
    peer: SocketAddr,
    id: ConnId,
    events: UnboundedSender<ServerEvent>,
    senders: Senders,
) {
    let ws = match tokio_tungstenite::accept_async(stream).await {
        Ok(ws) => ws,
        Err(e) => {
            log::warn!("handshake failed from {peer}: {e}");
            return;
        }
    };
    let (mut sink, mut source) = ws.split();
    let (tx, mut rx) = unbounded_channel::<Message>();
    senders.lock().unwrap().insert(id, tx);
    let _ = events.send(ServerEvent::Connected(id, peer));

    // Writer: pull from the channel into the socket.
    let writer = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            let is_close = matches!(msg, Message::Close(_));
            if sink.send(msg).await.is_err() || is_close {
                break;
            }
        }
        let _ = sink.close().await;
    });

    // Reader: forward binary messages until the peer goes away.
    while let Some(msg) = source.next().await {
        match msg {
            Ok(Message::Binary(data)) => {
                let _ = events.send(ServerEvent::Message(id, data.as_ref().to_vec()));
            }
            Ok(Message::Close(_)) | Err(_) => break,
            Ok(_) => {}
        }
    }

    senders.lock().unwrap().remove(&id);
    writer.abort();
    let _ = events.send(ServerEvent::Disconnected(id));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// End-to-end: native client transport talking to the tokio server.
    #[test]
    fn client_and_server_exchange_messages() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let mut server = WsServer::bind("127.0.0.1:0").await.unwrap();
            let url = format!("ws://{}", server.local_addr());

            // Run the native (blocking) client on a std thread.
            let client = std::thread::spawn(move || {
                use crate::transport::{
                    native::NativeWsTransport, ClientTransport, TransportEvent,
                };
                let mut t = NativeWsTransport::connect(&url).unwrap();
                t.send(vec![1, 2, 3]);
                // Poll until we get the echo back (with a deadline).
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
                loop {
                    for ev in t.poll() {
                        if let TransportEvent::Message(m) = ev {
                            return m;
                        }
                    }
                    assert!(std::time::Instant::now() < deadline, "echo timeout");
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
            });

            // Server side: expect connect + message, echo it back doubled.
            let mut got_conn = None;
            let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(5);
            loop {
                let ev = tokio::time::timeout_at(deadline, server.next_event())
                    .await
                    .expect("server event timeout")
                    .expect("server closed");
                match ev {
                    ServerEvent::Connected(id, _) => got_conn = Some(id),
                    ServerEvent::Message(id, data) => {
                        assert_eq!(Some(id), got_conn);
                        assert_eq!(data, vec![1, 2, 3]);
                        let doubled: Vec<u8> = data.iter().map(|b| b * 2).collect();
                        assert!(server.send(id, doubled));
                        break;
                    }
                    ServerEvent::Disconnected(_) => panic!("early disconnect"),
                }
            }

            let echoed = tokio::task::spawn_blocking(move || client.join().unwrap())
                .await
                .unwrap();
            assert_eq!(echoed, vec![2, 4, 6]);
        });
    }
}
