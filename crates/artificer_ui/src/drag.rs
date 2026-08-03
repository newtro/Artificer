//! The drag-gesture state machine, independent of what is being dragged.
//!
//! Every mouse drag has the same skeleton: a press CAPTURES a payload, cursor
//! travel past a threshold turns the press into a DRAG, and the release either
//! RESOLVES it or it CANCELS. What varies per game is what the payload is and
//! where it may land -- what must NOT vary is the discipline around the edges,
//! because that is where the bugs lived when a game hand-rolled this:
//!
//! - a lost cursor became `(0, 0)`, so leaving the window crossed the
//!   threshold by itself and a release out there aimed at whatever sat in the
//!   window corner;
//! - a release swallowed by focus loss (alt-tab mid-drag) left the gesture
//!   armed forever, resolving against bounds the player could no longer see;
//! - a press that survived its own release installed a stale payload later.
//!
//! [`DragTracker`] owns exactly that state and those rules. The caller feeds
//! it presses, cursor samples and releases, and reads back what the gesture
//! is; capture (what was pressed) and targeting (what is under the release)
//! stay with the caller, who is the only one who knows.

use bevy::prelude::*;

/// One in-flight drag gesture carrying a `Payload`.
#[derive(Debug, Clone)]
pub struct DragTracker<Payload> {
    pressed: Option<Payload>,
    origin: Vec2,
    active: bool,
    /// Cursor travel before a press counts as a drag rather than a click, in
    /// the same units as the cursor samples fed to [`DragTracker::track`].
    threshold: f32,
}

/// How a gesture ended, from [`DragTracker::release`].
#[derive(Debug, Clone, PartialEq)]
pub enum DragEnd<Payload> {
    /// The button came up without ever crossing the threshold.
    Click(Payload),
    /// The button came up after crossing it; the payload was DRAGGED here.
    Drop(Payload),
}

impl<Payload> DragTracker<Payload> {
    pub fn new(threshold: f32) -> Self {
        Self {
            pressed: None,
            origin: Vec2::ZERO,
            active: false,
            threshold,
        }
    }

    /// Begin a gesture: something under the cursor was pressed.
    pub fn press(&mut self, payload: Payload, at: Vec2) {
        self.pressed = Some(payload);
        self.origin = at;
        self.active = false;
    }

    /// Feed the current cursor position; the gesture becomes a DRAG once the
    /// cursor has travelled past the threshold.
    pub fn track(&mut self, cursor: Vec2) {
        if self.pressed.is_some() && !self.active && cursor.distance(self.origin) > self.threshold {
            self.active = true;
        }
    }

    /// The button came up. Consumes the gesture whole -- a press that
    /// survives its own release is how a stale payload gets used later.
    pub fn release(&mut self) -> Option<DragEnd<Payload>> {
        let payload = self.pressed.take()?;
        let was_dragging = self.active;
        self.active = false;
        Some(if was_dragging {
            DragEnd::Drop(payload)
        } else {
            DragEnd::Click(payload)
        })
    }

    /// Abandon the gesture entirely: lost cursor, lost focus, the surface it
    /// began on going away. Nothing is resolved and nothing survives.
    pub fn cancel(&mut self) {
        self.pressed = None;
        self.active = false;
    }

    /// What is currently pressed, drag or not.
    pub fn pressed(&self) -> Option<&Payload> {
        self.pressed.as_ref()
    }

    /// Whether the gesture has crossed the threshold and is a DRAG.
    pub fn dragging(&self) -> bool {
        self.active && self.pressed.is_some()
    }

    /// The payload in the air, only while actually dragging.
    pub fn carried(&self) -> Option<&Payload> {
        self.active.then_some(self.pressed.as_ref()).flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_press_is_a_click() {
        let mut drag: DragTracker<&str> = DragTracker::new(6.0);
        drag.press("cargo", Vec2::new(100.0, 100.0));
        drag.track(Vec2::new(102.0, 101.0));
        assert!(!drag.dragging(), "under the threshold is not a drag");
        assert_eq!(drag.release(), Some(DragEnd::Click("cargo")));
        assert!(drag.pressed().is_none(), "release consumes the gesture");
    }

    #[test]
    fn travel_past_the_threshold_makes_it_a_drop() {
        let mut drag: DragTracker<&str> = DragTracker::new(6.0);
        drag.press("cargo", Vec2::new(100.0, 100.0));
        drag.track(Vec2::new(140.0, 100.0));
        assert!(drag.dragging());
        assert_eq!(drag.carried(), Some(&"cargo"));
        assert_eq!(drag.release(), Some(DragEnd::Drop("cargo")));
    }

    #[test]
    fn cancel_leaves_nothing_to_resolve() {
        let mut drag: DragTracker<&str> = DragTracker::new(6.0);
        drag.press("cargo", Vec2::new(100.0, 100.0));
        drag.track(Vec2::new(200.0, 200.0));
        drag.cancel();
        assert!(!drag.dragging());
        assert_eq!(
            drag.release(),
            None,
            "a cancelled gesture must not resolve as click OR drop"
        );
    }

    /// The (0,0) bug this type exists to prevent: the CALLER cancels on a
    /// lost cursor instead of feeding a substitute, so travel is never
    /// measured against a point the cursor was never at.
    #[test]
    fn a_new_press_starts_clean_after_a_cancel() {
        let mut drag: DragTracker<&str> = DragTracker::new(6.0);
        drag.press("cargo", Vec2::new(100.0, 100.0));
        drag.cancel();
        drag.press("fuel", Vec2::new(500.0, 500.0));
        drag.track(Vec2::new(503.0, 500.0));
        assert!(
            !drag.dragging(),
            "the old origin must not leak into the new gesture"
        );
    }
}
