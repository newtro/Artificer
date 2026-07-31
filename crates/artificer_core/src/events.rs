//! Simple event primitives for headless simulation composition.
//!
//! `EventQueue` is a frame/tick-scoped mailbox: producers push during a
//! tick, consumers drain afterwards. Event RECORDING and replay live in
//! `artificer_testkit::replay` (EventLog + fold), which games use to prove
//! their persistent ledgers reproduce end state.

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
}
