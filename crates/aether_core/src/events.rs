//! Simple event primitives for headless simulation composition.
//!
//! `EventQueue` is a frame/tick-scoped mailbox: producers push during a tick,
//! consumers drain afterwards. `TimestampedLog` is an append-only record used
//! for replay and inspection (the engine's "commands, events, snapshots"
//! contract; the testkit builds replay on top of it).

use crate::tick::Tick;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::collections::VecDeque;

#[derive(Debug, Clone)]
pub struct EventQueue<T> {
    items: VecDeque<T>,
}

impl<T> Default for EventQueue<T> {
    fn default() -> Self {
        Self {
            items: VecDeque::new(),
        }
    }
}

impl<T> EventQueue<T> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, event: T) {
        self.items.push_back(event);
    }

    pub fn drain(&mut self) -> impl Iterator<Item = T> + '_ {
        self.items.drain(..)
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }
}

/// One recorded entry: the tick it happened on plus a serializable payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stamped<T> {
    pub tick: Tick,
    pub payload: T,
}

/// Append-only, serializable event log. JSONL on disk so logs are grep-able
/// and diff-able (machine-readable logs are an AI-first guardrail).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TimestampedLog<T> {
    entries: Vec<Stamped<T>>,
}

impl<T: Serialize + DeserializeOwned> TimestampedLog<T> {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub fn record(&mut self, tick: Tick, payload: T) {
        self.entries.push(Stamped { tick, payload });
    }

    pub fn entries(&self) -> &[Stamped<T>] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Serialize to JSON Lines (one entry per line).
    pub fn to_jsonl(&self) -> Result<String, serde_json::Error> {
        let mut out = String::new();
        for e in &self.entries {
            out.push_str(&serde_json::to_string(e)?);
            out.push('\n');
        }
        Ok(out)
    }

    /// Parse from JSON Lines, skipping blank lines.
    pub fn from_jsonl(input: &str) -> Result<Self, serde_json::Error> {
        let mut entries = Vec::new();
        for line in input.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            entries.push(serde_json::from_str::<Stamped<T>>(line)?);
        }
        Ok(Self { entries })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_drains_in_order() {
        let mut q = EventQueue::new();
        q.push(1);
        q.push(2);
        let got: Vec<i32> = q.drain().collect();
        assert_eq!(got, vec![1, 2]);
        assert!(q.is_empty());
    }

    #[test]
    fn log_round_trips_jsonl() {
        let mut log = TimestampedLog::new();
        log.record(Tick(1), "alpha".to_string());
        log.record(Tick(5), "beta".to_string());
        let text = log.to_jsonl().unwrap();
        let parsed = TimestampedLog::<String>::from_jsonl(&text).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed.entries()[1].tick, Tick(5));
        assert_eq!(parsed.entries()[1].payload, "beta");
    }
}
