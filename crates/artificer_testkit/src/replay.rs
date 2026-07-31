//! Event replay (§9.1: commands, events, replay).
//!
//! An [`EventLog`] is an ordered, tick-stamped record of domain events; a
//! [`replay`] fold re-derives state from it. Games use this to prove
//! their persistent ledgers are complete: fold the recorded events over
//! the initial state and the result must equal the live end state —
//! any drift means a mutation bypassed the log.
//!
//! Deliberately generic: the engine defines the recording/fold
//! discipline; the game defines the event type and the reducer.

use serde::{Deserialize, Serialize};

/// An ordered, tick-stamped event record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventLog<E> {
    entries: Vec<(u64, E)>,
}

impl<E> Default for EventLog<E> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
        }
    }
}

impl<E> EventLog<E> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append an event at a tick. Ticks may repeat (several events in one
    /// tick) but must never go backwards — replay depends on order.
    pub fn record(&mut self, tick: u64, event: E) {
        debug_assert!(
            self.entries.last().map(|(t, _)| *t <= tick).unwrap_or(true),
            "event log must be tick-ordered"
        );
        self.entries.push((tick, event));
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &(u64, E)> {
        self.entries.iter()
    }
}

/// Fold an event sequence over an initial state: the essence of replay.
/// `apply` must be pure — same initial state + same events = same result.
pub fn replay<S, E>(
    initial: S,
    events: impl IntoIterator<Item = E>,
    mut apply: impl FnMut(&mut S, E),
) -> S {
    let mut state = initial;
    for event in events {
        apply(&mut state, event);
    }
    state
}

/// Replay and compare against an observed end state; Ok(replayed) when
/// they agree, Err((replayed, observed)) when the log is incomplete.
pub fn verify_replay<S: PartialEq, E>(
    initial: S,
    events: impl IntoIterator<Item = E>,
    apply: impl FnMut(&mut S, E),
    observed: S,
) -> Result<S, (S, S)> {
    let replayed = replay(initial, events, apply);
    if replayed == observed {
        Ok(replayed)
    } else {
        Err((replayed, observed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    enum Bank {
        Deposit(i64),
        Withdraw(i64),
    }

    fn apply(balance: &mut i64, event: Bank) {
        match event {
            Bank::Deposit(n) => *balance += n,
            Bank::Withdraw(n) => *balance -= n,
        }
    }

    #[test]
    fn replay_reproduces_end_state() {
        let mut log = EventLog::new();
        log.record(1, Bank::Deposit(100));
        log.record(1, Bank::Withdraw(30));
        log.record(5, Bank::Deposit(7));
        let end = replay(0i64, log.iter().map(|(_, e)| e.clone()), apply);
        assert_eq!(end, 77);
        assert_eq!(log.len(), 3);
    }

    #[test]
    fn verify_replay_detects_missing_events() {
        let events = vec![Bank::Deposit(100)];
        // Observed state says 70: a Withdraw(30) is missing from the log.
        let result = verify_replay(0i64, events, apply, 70);
        assert_eq!(result, Err((100, 70)));

        let events = vec![Bank::Deposit(100), Bank::Withdraw(30)];
        assert_eq!(verify_replay(0i64, events, apply, 70), Ok(70));
    }
}
