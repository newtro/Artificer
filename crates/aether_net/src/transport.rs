//! Client-side transport abstraction with polling semantics that fit a
//! game frame loop: call [`ClientTransport::poll`] once per frame, drain
//! events, send at will. Implementations: native WebSocket (tungstenite,
//! non-blocking), browser WebSocket (web-sys), and an in-process loopback
//! (with an optional latency/jitter simulator wrapper) for tests and bots.

use std::collections::VecDeque;
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportEvent {
    /// Connection is established and messages may be sent.
    Opened,
    /// A complete binary message arrived.
    Message(Vec<u8>),
    /// Connection ended (reason is best-effort diagnostics).
    Closed(String),
}

pub trait ClientTransport {
    /// Queue a binary message for delivery. Silently dropped when closed.
    fn send(&mut self, bytes: Vec<u8>);
    /// Pump the connection; returns everything that happened since last poll.
    fn poll(&mut self) -> Vec<TransportEvent>;
    /// True between `Opened` and `Closed`.
    fn is_open(&self) -> bool;
}

/// In-process bidirectional transport pair: what one half sends, the other
/// receives. The backbone of automated-client tests.
pub struct LoopbackTransport {
    tx: Sender<Vec<u8>>,
    rx: Receiver<Vec<u8>>,
    opened_emitted: bool,
    open: bool,
}

impl LoopbackTransport {
    pub fn pair() -> (LoopbackTransport, LoopbackTransport) {
        let (tx_a, rx_b) = channel();
        let (tx_b, rx_a) = channel();
        (
            LoopbackTransport {
                tx: tx_a,
                rx: rx_a,
                opened_emitted: false,
                open: true,
            },
            LoopbackTransport {
                tx: tx_b,
                rx: rx_b,
                opened_emitted: false,
                open: true,
            },
        )
    }
}

impl ClientTransport for LoopbackTransport {
    fn send(&mut self, bytes: Vec<u8>) {
        if self.open && self.tx.send(bytes).is_err() {
            self.open = false;
        }
    }

    fn poll(&mut self) -> Vec<TransportEvent> {
        let mut events = Vec::new();
        if !self.opened_emitted {
            self.opened_emitted = true;
            events.push(TransportEvent::Opened);
        }
        loop {
            match self.rx.try_recv() {
                Ok(bytes) => events.push(TransportEvent::Message(bytes)),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    if self.open {
                        self.open = false;
                        events.push(TransportEvent::Closed("peer dropped".into()));
                    }
                    break;
                }
            }
        }
        events
    }

    fn is_open(&self) -> bool {
        self.open
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub mod latency {
    //! Latency/jitter simulation wrapper (flight-transport risk lab, plan §15).

    use super::{ClientTransport, TransportEvent};
    use aether_core::SeededRng;
    use std::collections::VecDeque;
    use std::time::{Duration, Instant};

    /// Wraps a transport, delaying every message in BOTH directions by
    /// `base_delay ± jitter` (uniform), deterministically from a seed.
    /// Total round trip ≈ 2 × base_delay.
    pub struct LatencyTransport<T: ClientTransport> {
        inner: T,
        base_delay: Duration,
        jitter: Duration,
        rng: SeededRng,
        outgoing: VecDeque<(Instant, Vec<u8>)>,
        incoming: VecDeque<(Instant, TransportEvent)>,
    }

    impl<T: ClientTransport> LatencyTransport<T> {
        pub fn new(inner: T, base_delay: Duration, jitter: Duration, seed: u64) -> Self {
            Self {
                inner,
                base_delay,
                jitter,
                rng: SeededRng::new(seed),
                outgoing: VecDeque::new(),
                incoming: VecDeque::new(),
            }
        }

        fn delay(&mut self) -> Duration {
            let jitter_secs = self.jitter.as_secs_f64();
            let offset = (self.rng.next_f64() * 2.0 - 1.0) * jitter_secs;
            let total = (self.base_delay.as_secs_f64() + offset).max(0.0);
            Duration::from_secs_f64(total)
        }
    }

    impl<T: ClientTransport> ClientTransport for LatencyTransport<T> {
        fn send(&mut self, bytes: Vec<u8>) {
            let due = Instant::now() + self.delay();
            self.outgoing.push_back((due, bytes));
        }

        fn poll(&mut self) -> Vec<TransportEvent> {
            let now = Instant::now();
            // Release due outgoing messages to the real transport.
            while let Some((due, _)) = self.outgoing.front() {
                if *due <= now {
                    let (_, bytes) = self.outgoing.pop_front().unwrap();
                    self.inner.send(bytes);
                } else {
                    break;
                }
            }
            // Stamp arrivals with a delivery time.
            for event in self.inner.poll() {
                match event {
                    TransportEvent::Message(_) => {
                        let due = now + self.delay();
                        self.incoming.push_back((due, event));
                    }
                    // Control events pass through undelayed.
                    other => self.incoming.push_front((now, other)),
                }
            }
            // Release due incoming.
            let mut events = Vec::new();
            while let Some((due, _)) = self.incoming.front() {
                if *due <= now {
                    events.push(self.incoming.pop_front().unwrap().1);
                } else {
                    break;
                }
            }
            events
        }

        fn is_open(&self) -> bool {
            self.inner.is_open()
        }
    }
}

#[cfg(all(feature = "client-native", not(target_arch = "wasm32")))]
pub mod native {
    //! Non-blocking native WebSocket client over tungstenite.

    use super::{ClientTransport, TransportEvent};
    use std::net::TcpStream;
    use tungstenite::stream::MaybeTlsStream;
    use tungstenite::{Message, WebSocket};

    pub struct NativeWsTransport {
        socket: Option<WebSocket<MaybeTlsStream<TcpStream>>>,
        open: bool,
        opened_emitted: bool,
        error: Option<String>,
    }

    impl NativeWsTransport {
        /// Connect synchronously (fast on LAN/localhost), then switch the
        /// stream to non-blocking for frame-loop polling.
        pub fn connect(url: &str) -> Result<Self, String> {
            let (mut socket, _resp) =
                tungstenite::connect(url).map_err(|e| format!("connect {url}: {e}"))?;
            match socket.get_mut() {
                MaybeTlsStream::Plain(stream) => stream
                    .set_nonblocking(true)
                    .map_err(|e| format!("set_nonblocking: {e}"))?,
                _ => return Err("TLS streams not supported in MVP transport".into()),
            }
            Ok(Self {
                socket: Some(socket),
                open: true,
                opened_emitted: false,
                error: None,
            })
        }
    }

    impl ClientTransport for NativeWsTransport {
        fn send(&mut self, bytes: Vec<u8>) {
            if let Some(socket) = self.socket.as_mut() {
                if let Err(e) = socket.send(Message::binary(bytes)) {
                    // WouldBlock on send is a full buffer; treat others fatal.
                    if !matches!(&e, tungstenite::Error::Io(io) if io.kind() == std::io::ErrorKind::WouldBlock)
                    {
                        self.error = Some(e.to_string());
                        self.open = false;
                    }
                }
            }
        }

        fn poll(&mut self) -> Vec<TransportEvent> {
            let mut events = Vec::new();
            if !self.opened_emitted && self.open {
                self.opened_emitted = true;
                events.push(TransportEvent::Opened);
            }
            let Some(socket) = self.socket.as_mut() else {
                return events;
            };
            loop {
                match socket.read() {
                    Ok(Message::Binary(data)) => {
                        events.push(TransportEvent::Message(data.as_ref().to_vec()));
                    }
                    Ok(Message::Close(frame)) => {
                        self.open = false;
                        let reason = frame
                            .map(|f| f.reason.to_string())
                            .unwrap_or_else(|| "closed".to_string());
                        events.push(TransportEvent::Closed(reason));
                        self.socket = None;
                        break;
                    }
                    Ok(_) => {} // text/ping/pong handled by tungstenite
                    Err(tungstenite::Error::Io(io))
                        if io.kind() == std::io::ErrorKind::WouldBlock =>
                    {
                        break;
                    }
                    Err(e) => {
                        self.open = false;
                        events.push(TransportEvent::Closed(e.to_string()));
                        self.socket = None;
                        break;
                    }
                }
            }
            events
        }

        fn is_open(&self) -> bool {
            self.open
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub mod wasm {
    //! Browser WebSocket client over web-sys.

    use super::{ClientTransport, TransportEvent};
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::rc::Rc;
    use wasm_bindgen::prelude::*;
    use wasm_bindgen::JsCast;
    use web_sys::{BinaryType, CloseEvent, MessageEvent, WebSocket};

    pub struct WasmWsTransport {
        socket: WebSocket,
        queue: Rc<RefCell<VecDeque<TransportEvent>>>,
        open: Rc<RefCell<bool>>,
        // Keep callbacks alive for the socket's lifetime.
        _callbacks: Vec<Closure<dyn FnMut(JsValue)>>,
    }

    impl WasmWsTransport {
        pub fn connect(url: &str) -> Result<Self, String> {
            let socket = WebSocket::new(url).map_err(|e| format!("WebSocket::new: {e:?}"))?;
            socket.set_binary_type(BinaryType::Arraybuffer);

            let queue: Rc<RefCell<VecDeque<TransportEvent>>> =
                Rc::new(RefCell::new(VecDeque::new()));
            let open = Rc::new(RefCell::new(false));
            let mut callbacks: Vec<Closure<dyn FnMut(JsValue)>> = Vec::new();

            {
                let queue = queue.clone();
                let open = open.clone();
                let cb = Closure::wrap(Box::new(move |_e: JsValue| {
                    *open.borrow_mut() = true;
                    queue.borrow_mut().push_back(TransportEvent::Opened);
                }) as Box<dyn FnMut(JsValue)>);
                socket.set_onopen(Some(cb.as_ref().unchecked_ref()));
                callbacks.push(cb);
            }
            {
                let queue = queue.clone();
                let cb = Closure::wrap(Box::new(move |e: JsValue| {
                    let e: MessageEvent = e.unchecked_into();
                    if let Ok(buf) = e.data().dyn_into::<js_sys::ArrayBuffer>() {
                        let bytes = js_sys::Uint8Array::new(&buf).to_vec();
                        queue.borrow_mut().push_back(TransportEvent::Message(bytes));
                    }
                }) as Box<dyn FnMut(JsValue)>);
                socket.set_onmessage(Some(cb.as_ref().unchecked_ref()));
                callbacks.push(cb);
            }
            {
                let queue = queue.clone();
                let open = open.clone();
                let cb = Closure::wrap(Box::new(move |e: JsValue| {
                    *open.borrow_mut() = false;
                    let reason = e
                        .dyn_into::<CloseEvent>()
                        .map(|c| format!("code {}: {}", c.code(), c.reason()))
                        .unwrap_or_else(|_| "closed".to_string());
                    queue.borrow_mut().push_back(TransportEvent::Closed(reason));
                }) as Box<dyn FnMut(JsValue)>);
                socket.set_onclose(Some(cb.as_ref().unchecked_ref()));
                callbacks.push(cb);
            }
            {
                let queue = queue.clone();
                let open = open.clone();
                let cb = Closure::wrap(Box::new(move |_e: JsValue| {
                    if *open.borrow() {
                        *open.borrow_mut() = false;
                        queue
                            .borrow_mut()
                            .push_back(TransportEvent::Closed("socket error".into()));
                    }
                }) as Box<dyn FnMut(JsValue)>);
                socket.set_onerror(Some(cb.as_ref().unchecked_ref()));
                callbacks.push(cb);
            }

            Ok(Self {
                socket,
                queue,
                open,
                _callbacks: callbacks,
            })
        }
    }

    impl ClientTransport for WasmWsTransport {
        fn send(&mut self, bytes: Vec<u8>) {
            if *self.open.borrow() {
                let _ = self.socket.send_with_u8_array(&bytes);
            }
        }

        fn poll(&mut self) -> Vec<TransportEvent> {
            self.queue.borrow_mut().drain(..).collect()
        }

        fn is_open(&self) -> bool {
            *self.open.borrow()
        }
    }
}

// Silence "unused import" when only some features are enabled.
#[allow(unused)]
fn _assert_object_safe(_: &dyn ClientTransport) {}

#[allow(unused)]
type _Unused = VecDeque<u8>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_round_trips() {
        let (mut a, mut b) = LoopbackTransport::pair();
        assert_eq!(a.poll(), vec![TransportEvent::Opened]);
        assert_eq!(b.poll(), vec![TransportEvent::Opened]);
        a.send(vec![1, 2, 3]);
        b.send(vec![9]);
        assert_eq!(b.poll(), vec![TransportEvent::Message(vec![1, 2, 3])]);
        assert_eq!(a.poll(), vec![TransportEvent::Message(vec![9])]);
    }

    #[test]
    fn loopback_detects_peer_drop() {
        let (mut a, b) = LoopbackTransport::pair();
        drop(b);
        a.poll();
        a.send(vec![1]);
        let events = a.poll();
        assert!(
            !a.is_open()
                || events
                    .iter()
                    .any(|e| matches!(e, TransportEvent::Closed(_)))
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn latency_wrapper_delays_delivery() {
        use std::time::Duration;
        let (a, mut b) = LoopbackTransport::pair();
        let mut lag =
            latency::LatencyTransport::new(a, Duration::from_millis(60), Duration::ZERO, 42);
        b.poll();
        lag.poll();
        lag.send(vec![7]);
        // Immediately: nothing released yet.
        lag.poll();
        assert!(b.poll().is_empty(), "message must not arrive instantly");
        std::thread::sleep(Duration::from_millis(80));
        lag.poll(); // releases the due outgoing message
        let events = b.poll();
        assert_eq!(events, vec![TransportEvent::Message(vec![7])]);
    }
}
