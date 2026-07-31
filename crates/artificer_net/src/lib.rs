//! Network foundations: versioned wire codec, WebSocket transports for
//! server / native client / browser client, an in-process loopback with a
//! latency-and-jitter simulator, and generic prediction buffers.
//!
//! The engine owns transport and prediction *machinery*; games define their
//! own message types and re-simulation. Everything here is game-agnostic.

pub mod codec;
pub mod prediction;
pub mod transport;

#[cfg(feature = "server")]
pub mod server;

pub use codec::{decode_versioned, encode_versioned, CodecError};
pub use transport::{ClientTransport, LoopbackTransport, TransportEvent};

#[cfg(all(feature = "client-native", not(target_arch = "wasm32")))]
pub use transport::native::NativeWsTransport;

#[cfg(target_arch = "wasm32")]
pub use transport::wasm::WasmWsTransport;

#[cfg(not(target_arch = "wasm32"))]
pub use transport::latency::LatencyTransport;
