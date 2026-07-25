//! Generic client-side prediction machinery.
//!
//! [`InputBuffer`] tracks sent-but-unacknowledged inputs for reconciliation
//! replay. [`SnapshotInterp`] holds a short history of authoritative
//! snapshots and answers "which two snapshots straddle the render time, and
//! how far between them are we" — the game lerps its own state types.

use std::collections::VecDeque;

/// Sequence-numbered outgoing inputs awaiting server acknowledgement.
#[derive(Debug, Clone)]
pub struct InputBuffer<A> {
    next_seq: u32,
    pending: VecDeque<(u32, A)>,
    capacity: usize,
}

impl<A> Default for InputBuffer<A> {
    fn default() -> Self {
        Self::new(240)
    }
}

impl<A> InputBuffer<A> {
    /// `capacity`: safety cap on outstanding inputs (a stalled server must
    /// not grow the buffer forever; oldest entries drop first).
    pub fn new(capacity: usize) -> Self {
        Self {
            next_seq: 1,
            pending: VecDeque::new(),
            capacity: capacity.max(1),
        }
    }

    /// Store an input and return the sequence number to send with it.
    pub fn push(&mut self, action: A) -> u32 {
        let seq = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(1);
        self.pending.push_back((seq, action));
        while self.pending.len() > self.capacity {
            self.pending.pop_front();
        }
        seq
    }

    /// Server confirmed processing everything up to and including `seq`.
    pub fn ack(&mut self, seq: u32) {
        while let Some((s, _)) = self.pending.front() {
            if *s <= seq {
                self.pending.pop_front();
            } else {
                break;
            }
        }
    }

    /// Inputs the server has not yet confirmed, oldest first — the
    /// reconciliation replay set.
    pub fn unacked(&self) -> impl Iterator<Item = &(u32, A)> {
        self.pending.iter()
    }

    pub fn len(&self) -> usize {
        self.pending.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}

/// Snapshot pair straddling the requested playback time.
#[derive(Debug)]
pub struct InterpSample<'a, S> {
    pub from: &'a S,
    pub to: &'a S,
    /// 0.0 = exactly `from`, 1.0 = exactly `to`.
    pub alpha: f32,
}

/// History of authoritative snapshots keyed by server tick, sampled with an
/// interpolation delay so remote entities render smoothly between updates.
#[derive(Debug, Clone)]
pub struct SnapshotInterp<S> {
    buffer: VecDeque<(u64, S)>,
    capacity: usize,
}

impl<S> Default for SnapshotInterp<S> {
    fn default() -> Self {
        Self::new(32)
    }
}

impl<S> SnapshotInterp<S> {
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: VecDeque::new(),
            capacity: capacity.max(2),
        }
    }

    /// Insert a snapshot; out-of-order arrivals older than the newest are
    /// ignored (WebSocket is ordered, but reconnects can reorder logically).
    pub fn push(&mut self, tick: u64, snapshot: S) {
        if let Some((newest, _)) = self.buffer.back() {
            if tick <= *newest {
                return;
            }
        }
        self.buffer.push_back((tick, snapshot));
        while self.buffer.len() > self.capacity {
            self.buffer.pop_front();
        }
    }

    pub fn latest(&self) -> Option<(u64, &S)> {
        self.buffer.back().map(|(t, s)| (*t, s))
    }

    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// Sample at `render_tick` (fractional server ticks, already including
    /// the caller's interpolation delay). Clamps to the buffered range.
    pub fn sample(&self, render_tick: f64) -> Option<InterpSample<'_, S>> {
        if self.buffer.is_empty() {
            return None;
        }
        if self.buffer.len() == 1 {
            let (_, s) = &self.buffer[0];
            return Some(InterpSample {
                from: s,
                to: s,
                alpha: 0.0,
            });
        }
        // Clamp below range.
        if render_tick <= self.buffer[0].0 as f64 {
            let (_, s) = &self.buffer[0];
            return Some(InterpSample {
                from: s,
                to: s,
                alpha: 0.0,
            });
        }
        // Find the straddling pair.
        for window in 0..self.buffer.len() - 1 {
            let (t0, _) = self.buffer[window];
            let (t1, _) = self.buffer[window + 1];
            if render_tick >= t0 as f64 && render_tick <= t1 as f64 {
                let span = (t1 - t0) as f64;
                let alpha = if span > 0.0 {
                    ((render_tick - t0 as f64) / span) as f32
                } else {
                    0.0
                };
                let from = &self.buffer[window].1;
                let to = &self.buffer[window + 1].1;
                return Some(InterpSample { from, to, alpha });
            }
        }
        // Beyond the newest: hold the last snapshot (no extrapolation).
        let (_, s) = self.buffer.back().unwrap();
        Some(InterpSample {
            from: s,
            to: s,
            alpha: 0.0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_buffer_acks_prefix() {
        let mut buf = InputBuffer::new(100);
        let s1 = buf.push("a");
        let s2 = buf.push("b");
        let s3 = buf.push("c");
        assert_eq!((s1, s2, s3), (1, 2, 3));
        buf.ack(2);
        let rest: Vec<u32> = buf.unacked().map(|(s, _)| *s).collect();
        assert_eq!(rest, vec![3]);
    }

    #[test]
    fn input_buffer_caps_growth() {
        let mut buf = InputBuffer::new(4);
        for i in 0..100 {
            buf.push(i);
        }
        assert_eq!(buf.len(), 4);
    }

    #[test]
    fn interp_samples_between_snapshots() {
        let mut interp = SnapshotInterp::new(8);
        interp.push(10, 100.0f32);
        interp.push(20, 200.0f32);
        let s = interp.sample(15.0).unwrap();
        assert_eq!(*s.from, 100.0);
        assert_eq!(*s.to, 200.0);
        assert!((s.alpha - 0.5).abs() < 1e-6);
    }

    #[test]
    fn interp_clamps_and_holds() {
        let mut interp = SnapshotInterp::new(8);
        interp.push(10, 1.0f32);
        interp.push(20, 2.0f32);
        assert_eq!(*interp.sample(5.0).unwrap().from, 1.0);
        let held = interp.sample(99.0).unwrap();
        assert_eq!(*held.from, 2.0);
        assert_eq!(held.alpha, 0.0);
    }

    #[test]
    fn out_of_order_pushes_ignored() {
        let mut interp = SnapshotInterp::new(8);
        interp.push(10, 1.0f32);
        interp.push(20, 2.0f32);
        interp.push(15, 99.0f32);
        assert_eq!(interp.len(), 2);
        assert_eq!(interp.latest().unwrap().0, 20);
    }
}
