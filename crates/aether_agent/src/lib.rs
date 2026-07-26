//! aether_agent — the headless agent SDK (plan §9.2).
//!
//! Games are only half the promise of an engine; the other half is the
//! swarm of automated participants — AI actors, load bots, soak harnesses,
//! tutorial ghosts — that PLAY those games without a window. This crate is
//! the harness they all share: a real transport, a fixed-rate think loop,
//! and a clean seam between "how bytes move" (the engine's job) and "what
//! the agent wants" (the game's job).
//!
//! The SDK is deliberately byte-oriented: it never sees the game's
//! protocol types, so any aether game — and any protocol version — can
//! drive agents through it. Parity is structural: an agent gets no side
//! channel, only the same transport a human client uses.

use aether_net::{ClientTransport, TransportEvent};
use std::time::{Duration, Instant};

/// A game-side protocol adapter: owns the game's client state machine
/// (handshake, prediction, intel, …) and the agent's decision layers.
/// The harness calls it; it answers with outbound protocol bytes.
pub trait HeadlessClient {
    /// Transport opened — return the handshake (e.g. a Hello message).
    fn on_open(&mut self) -> Vec<Vec<u8>>;

    /// One inbound server message (perception).
    fn on_message(&mut self, bytes: &[u8], now_seconds: f64);

    /// Transport closed by the far side.
    fn on_closed(&mut self, reason: String);

    /// One fixed tick: perceive → think → act. Returns messages to send.
    fn on_tick(&mut self, now_seconds: f64) -> Vec<Vec<u8>>;

    /// Return Some(reason) to end the session from the agent's side
    /// (goal reached, rejected by server, gave up).
    fn finished(&self) -> Option<String>;
}

/// How an agent session ended, plus basic loop accounting.
#[derive(Debug, Clone)]
pub struct AgentOutcome {
    /// Human-readable end cause ("duration elapsed", "closed: …", or the
    /// client's own `finished()` reason).
    pub ended: String,
    /// Fixed ticks actually executed.
    pub ticks: u64,
    /// Server messages delivered to the client.
    pub messages_in: u64,
    /// Client messages sent to the server.
    pub messages_out: u64,
    /// Wall-clock session length.
    pub elapsed: Duration,
}

/// Fixed-rate agent loop configuration.
#[derive(Debug, Clone)]
pub struct AgentLoop {
    /// Think/act frequency (Hz). Matches the game's input rate in
    /// real-time use; can run far faster against accelerated servers.
    pub tick_rate: f64,
    /// Hard wall-clock cap for the session.
    pub max_duration: Duration,
    /// Sleep granularity between polls. Smaller = lower added latency,
    /// higher CPU. 2ms suits real-time; 0 spins for accelerated sims.
    pub idle_sleep: Duration,
}

impl Default for AgentLoop {
    fn default() -> Self {
        Self {
            tick_rate: 30.0,
            max_duration: Duration::from_secs(u64::MAX / 4),
            idle_sleep: Duration::from_millis(2),
        }
    }
}

impl AgentLoop {
    /// Drive one agent session to completion: pump the transport, deliver
    /// perception, run fixed-rate think ticks, send actions. Blocking;
    /// callers run one thread (or one accelerated loop) per agent.
    pub fn run(
        &self,
        mut transport: Box<dyn ClientTransport>,
        client: &mut dyn HeadlessClient,
    ) -> AgentOutcome {
        let start = Instant::now();
        let tick = Duration::from_secs_f64(1.0 / self.tick_rate.max(0.001));
        let mut next_tick = Instant::now();
        let mut outcome = AgentOutcome {
            ended: "duration elapsed".to_string(),
            ticks: 0,
            messages_in: 0,
            messages_out: 0,
            elapsed: Duration::ZERO,
        };

        'session: while start.elapsed() < self.max_duration {
            let now = start.elapsed().as_secs_f64();
            for event in transport.poll() {
                match event {
                    TransportEvent::Opened => {
                        for msg in client.on_open() {
                            outcome.messages_out += 1;
                            transport.send(msg);
                        }
                    }
                    TransportEvent::Message(bytes) => {
                        outcome.messages_in += 1;
                        client.on_message(&bytes, now);
                    }
                    TransportEvent::Closed(reason) => {
                        client.on_closed(reason.clone());
                        // A goal reached in the same poll batch as the
                        // disconnect is still a reached goal: the agent's
                        // own verdict outranks how the socket ended.
                        outcome.ended = client
                            .finished()
                            .unwrap_or_else(|| format!("closed: {reason}"));
                        break 'session;
                    }
                }
            }
            if let Some(reason) = client.finished() {
                outcome.ended = reason;
                break;
            }

            if Instant::now() >= next_tick {
                next_tick += tick;
                outcome.ticks += 1;
                for msg in client.on_tick(now) {
                    outcome.messages_out += 1;
                    transport.send(msg);
                }
            }
            if !self.idle_sleep.is_zero() {
                std::thread::sleep(self.idle_sleep);
            }
        }

        outcome.elapsed = start.elapsed();
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_net::LoopbackTransport;

    /// Scripted client: greets, echoes every message once, quits after N
    /// messages have come back.
    struct EchoClient {
        seen: u64,
        quit_after: u64,
        pending_echo: Vec<Vec<u8>>,
        closed: Option<String>,
    }

    impl HeadlessClient for EchoClient {
        fn on_open(&mut self) -> Vec<Vec<u8>> {
            vec![b"hello".to_vec()]
        }

        fn on_message(&mut self, bytes: &[u8], _now: f64) {
            self.seen += 1;
            self.pending_echo.push(bytes.to_vec());
        }

        fn on_closed(&mut self, reason: String) {
            self.closed = Some(reason);
        }

        fn on_tick(&mut self, _now: f64) -> Vec<Vec<u8>> {
            std::mem::take(&mut self.pending_echo)
        }

        fn finished(&self) -> Option<String> {
            (self.seen >= self.quit_after).then(|| "goal reached".to_string())
        }
    }

    #[test]
    fn loop_pumps_handshake_perception_and_actions() {
        let (client_side, server_side) = LoopbackTransport::pair();
        // The "server": reflect everything the agent sends, three times.
        let server = std::thread::spawn(move || {
            let mut server_side = server_side;
            let mut reflected = 0;
            while reflected < 3 {
                for event in server_side.poll() {
                    if let TransportEvent::Message(bytes) = event {
                        server_side.send(bytes);
                        reflected += 1;
                    }
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        });

        let mut agent = EchoClient {
            seen: 0,
            quit_after: 3,
            pending_echo: Vec::new(),
            closed: None,
        };
        let outcome = AgentLoop {
            tick_rate: 120.0,
            max_duration: Duration::from_secs(5),
            idle_sleep: Duration::from_millis(1),
        }
        .run(Box::new(client_side), &mut agent);
        server.join().unwrap();

        assert_eq!(outcome.ended, "goal reached");
        assert_eq!(outcome.messages_in, 3);
        // hello + at least the echoes that made it back before quitting.
        assert!(outcome.messages_out >= 3);
        assert!(outcome.ticks > 0);
    }

    #[test]
    fn loop_respects_max_duration() {
        let (client_side, _server_side) = LoopbackTransport::pair();
        let mut agent = EchoClient {
            seen: 0,
            quit_after: u64::MAX,
            pending_echo: Vec::new(),
            closed: None,
        };
        let outcome = AgentLoop {
            tick_rate: 60.0,
            max_duration: Duration::from_millis(80),
            idle_sleep: Duration::from_millis(1),
        }
        .run(Box::new(client_side), &mut agent);
        assert_eq!(outcome.ended, "duration elapsed");
        assert!(outcome.elapsed >= Duration::from_millis(80));
    }
}
