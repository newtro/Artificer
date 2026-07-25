//! Platform-neutral input: a per-frame [`InputState`] snapshot plus a
//! logical [`ActionMap`] layered on top.
//!
//! Adapters (the Bevy client, scripted tests, replay) *inject* raw state;
//! game code reads logical actions. Because injection is plain method calls,
//! headless tests can drive the exact same game input paths as a real player.

use glam::Vec2;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::hash::Hash;

/// Physical keys the engine understands (extend as games need them).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Key {
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    I,
    J,
    K,
    L,
    M,
    N,
    O,
    P,
    Q,
    R,
    S,
    T,
    U,
    V,
    W,
    X,
    Y,
    Z,
    Digit0,
    Digit1,
    Digit2,
    Digit3,
    Digit4,
    Digit5,
    Digit6,
    Digit7,
    Digit8,
    Digit9,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    Up,
    Down,
    Left,
    Right,
    Space,
    ShiftLeft,
    ControlLeft,
    AltLeft,
    Tab,
    Enter,
    Escape,
    Backspace,
    Minus,
    Equals,
    BracketLeft,
    BracketRight,
    Comma,
    Period,
    Slash,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

/// Raw input snapshot for one frame.
#[derive(Debug, Clone, Default)]
pub struct InputState {
    pressed: HashSet<Key>,
    just_pressed: HashSet<Key>,
    just_released: HashSet<Key>,
    mouse_pressed: HashSet<MouseButton>,
    mouse_just_pressed: HashSet<MouseButton>,
    mouse_just_released: HashSet<MouseButton>,
    /// Pointer position in logical window pixels (top-left origin).
    pub mouse_position: Vec2,
    /// Pointer movement since last frame.
    pub mouse_delta: Vec2,
    /// Scroll wheel movement since last frame (lines; +y = away from user).
    pub wheel_delta: f32,
}

impl InputState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Clear per-frame deltas. Adapters call this at the start of each frame
    /// before injecting fresh events.
    pub fn begin_frame(&mut self) {
        self.just_pressed.clear();
        self.just_released.clear();
        self.mouse_just_pressed.clear();
        self.mouse_just_released.clear();
        self.mouse_delta = Vec2::ZERO;
        self.wheel_delta = 0.0;
    }

    // -- injection (adapters, tests) --

    pub fn press(&mut self, key: Key) {
        if self.pressed.insert(key) {
            self.just_pressed.insert(key);
        }
    }

    pub fn release(&mut self, key: Key) {
        if self.pressed.remove(&key) {
            self.just_released.insert(key);
        }
    }

    pub fn press_mouse(&mut self, button: MouseButton) {
        if self.mouse_pressed.insert(button) {
            self.mouse_just_pressed.insert(button);
        }
    }

    pub fn release_mouse(&mut self, button: MouseButton) {
        if self.mouse_pressed.remove(&button) {
            self.mouse_just_released.insert(button);
        }
    }

    pub fn add_mouse_delta(&mut self, delta: Vec2) {
        self.mouse_delta += delta;
    }

    pub fn add_wheel(&mut self, delta: f32) {
        self.wheel_delta += delta;
    }

    /// Release everything (window focus loss) so keys don't stick.
    pub fn clear_all(&mut self) {
        let held: Vec<Key> = self.pressed.iter().copied().collect();
        for k in held {
            self.release(k);
        }
        let held: Vec<MouseButton> = self.mouse_pressed.iter().copied().collect();
        for b in held {
            self.release_mouse(b);
        }
    }

    // -- queries (game code) --

    pub fn is_pressed(&self, key: Key) -> bool {
        self.pressed.contains(&key)
    }

    pub fn just_pressed(&self, key: Key) -> bool {
        self.just_pressed.contains(&key)
    }

    pub fn just_released(&self, key: Key) -> bool {
        self.just_released.contains(&key)
    }

    pub fn mouse_is_pressed(&self, b: MouseButton) -> bool {
        self.mouse_pressed.contains(&b)
    }

    pub fn mouse_just_pressed(&self, b: MouseButton) -> bool {
        self.mouse_just_pressed.contains(&b)
    }

    /// Axis helper: +1 if `pos` held, -1 if `neg` held, 0 otherwise.
    pub fn axis(&self, pos: Key, neg: Key) -> f32 {
        (self.is_pressed(pos) as i32 - self.is_pressed(neg) as i32) as f32
    }
}

/// Maps logical game actions to one or more keys.
#[derive(Debug, Clone)]
pub struct ActionMap<A: Copy + Eq + Hash> {
    bindings: HashMap<A, Vec<Key>>,
}

impl<A: Copy + Eq + Hash> Default for ActionMap<A> {
    fn default() -> Self {
        Self {
            bindings: HashMap::new(),
        }
    }
}

impl<A: Copy + Eq + Hash> ActionMap<A> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn bind(mut self, action: A, key: Key) -> Self {
        self.bindings.entry(action).or_default().push(key);
        self
    }

    pub fn is_active(&self, action: A, input: &InputState) -> bool {
        self.bindings
            .get(&action)
            .map(|keys| keys.iter().any(|k| input.is_pressed(*k)))
            .unwrap_or(false)
    }

    pub fn just_activated(&self, action: A, input: &InputState) -> bool {
        self.bindings
            .get(&action)
            .map(|keys| keys.iter().any(|k| input.just_pressed(*k)))
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    enum Act {
        Thrust,
        Fire,
    }

    #[test]
    fn press_release_cycle() {
        let mut s = InputState::new();
        s.press(Key::W);
        assert!(s.is_pressed(Key::W) && s.just_pressed(Key::W));
        s.begin_frame();
        assert!(s.is_pressed(Key::W) && !s.just_pressed(Key::W));
        s.release(Key::W);
        assert!(!s.is_pressed(Key::W) && s.just_released(Key::W));
    }

    #[test]
    fn axis_combines_keys() {
        let mut s = InputState::new();
        s.press(Key::W);
        assert_eq!(s.axis(Key::W, Key::S), 1.0);
        s.press(Key::S);
        assert_eq!(s.axis(Key::W, Key::S), 0.0);
    }

    #[test]
    fn action_map_matches_any_binding() {
        let map = ActionMap::new()
            .bind(Act::Thrust, Key::W)
            .bind(Act::Thrust, Key::Up)
            .bind(Act::Fire, Key::Space);
        let mut s = InputState::new();
        s.press(Key::Up);
        assert!(map.is_active(Act::Thrust, &s));
        assert!(!map.is_active(Act::Fire, &s));
    }

    #[test]
    fn clear_all_releases_held_keys() {
        let mut s = InputState::new();
        s.press(Key::W);
        s.press_mouse(MouseButton::Left);
        s.begin_frame();
        s.clear_all();
        assert!(!s.is_pressed(Key::W));
        assert!(s.just_released(Key::W));
        assert!(!s.mouse_is_pressed(MouseButton::Left));
    }
}
