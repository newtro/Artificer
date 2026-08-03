//! Draggable dividers that resize the panel beside them.
//!
//! A fixed-width sidebar is a guess about how much room its contents deserve,
//! and it is wrong as soon as the content changes — an asset grid wants to be
//! wide while you browse and narrow while you inspect. Putting a [`Splitter`]
//! between two panels lets that be decided at the time it matters.
//!
//! The splitter resizes ONE target's `Node::width`; whatever sits next to it
//! is expected to take up the slack with `flex_grow`, which is how a flex row
//! wants to be driven anyway.

use bevy::prelude::*;
use bevy::ui::ComputedNode;

/// A divider that resizes `target` as it is dragged left and right.
#[derive(Component, Debug, Clone, Copy)]
pub struct Splitter {
    /// The node whose `width` this drag sets. Must be `Val::Px`; a percentage
    /// or auto width has nothing for a pixel delta to act on.
    pub target: Entity,
    pub min: f32,
    pub max: f32,
    /// True when the target lies to the RIGHT of the divider, so dragging
    /// right makes it narrower rather than wider.
    pub invert: bool,
}

impl Splitter {
    pub fn new(target: Entity, min: f32, max: f32) -> Self {
        Self {
            target,
            min,
            max,
            invert: false,
        }
    }

    /// For a divider whose target is on its right.
    pub fn inverted(target: Entity, min: f32, max: f32) -> Self {
        Self {
            invert: true,
            ..Self::new(target, min, max)
        }
    }

    /// Width the target should take after moving the divider by `dx` logical
    /// pixels.
    pub fn resolve(&self, current: f32, dx: f32) -> f32 {
        let delta = if self.invert { -dx } else { dx };
        (current + delta).clamp(self.min, self.max)
    }
}

/// Which splitter is being dragged, and where the pointer was last frame.
///
/// A grab has to be remembered: `Interaction` stops reporting the divider the
/// moment the pointer slides off it, and a divider you lose the instant you
/// drag faster than the layout follows is worse than no divider at all.
#[derive(Resource, Default)]
pub struct SplitterDrag {
    active: Option<Entity>,
    last_x: f32,
}

impl SplitterDrag {
    pub fn is_dragging(&self) -> bool {
        self.active.is_some()
    }
}

pub fn drive_splitters(
    mouse: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
    splitters: Query<(Entity, &Interaction, &Splitter)>,
    computed: Query<&ComputedNode>,
    mut nodes: Query<&mut Node>,
    mut drag: ResMut<SplitterDrag>,
) {
    let cursor = windows.iter().next().and_then(|w| w.cursor_position());

    if !mouse.pressed(MouseButton::Left) {
        drag.active = None;
        return;
    }
    let Some(cursor) = cursor else {
        drag.active = None;
        return;
    };

    // Start a grab only on a divider actually under the pointer.
    if drag.active.is_none() {
        if !mouse.just_pressed(MouseButton::Left) {
            return;
        }
        let Some((entity, _, _)) = splitters
            .iter()
            .find(|(_, interaction, _)| **interaction != Interaction::None)
        else {
            return;
        };
        drag.active = Some(entity);
        drag.last_x = cursor.x;
        return;
    }

    let Some(active) = drag.active else {
        return;
    };
    let dx = cursor.x - drag.last_x;
    drag.last_x = cursor.x;
    if dx == 0.0 {
        return;
    }
    let Ok((_, _, splitter)) = splitters.get(active) else {
        drag.active = None;
        return;
    };
    // Measured, not remembered: reading the width back each frame keeps the
    // divider stuck to the pointer even when a clamp refused part of a move.
    let current = computed
        .get(splitter.target)
        .map(|node| node.size().x * node.inverse_scale_factor())
        .unwrap_or(splitter.min);
    if let Ok(mut node) = nodes.get_mut(splitter.target) {
        node.width = Val::Px(splitter.resolve(current, dx));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn splitter() -> Splitter {
        Splitter::new(Entity::from_raw(1), 100.0, 500.0)
    }

    #[test]
    fn dragging_right_widens_a_target_on_the_left() {
        assert_eq!(splitter().resolve(200.0, 30.0), 230.0);
    }

    #[test]
    fn an_inverted_splitter_narrows_its_target_when_dragged_right() {
        // The detail pane sits to the RIGHT of its divider, so pushing the
        // divider right must give its space to the grid, not take more.
        let s = Splitter::inverted(Entity::from_raw(1), 100.0, 500.0);
        assert_eq!(s.resolve(300.0, 40.0), 260.0);
        assert_eq!(s.resolve(300.0, -40.0), 340.0);
    }

    #[test]
    fn width_is_clamped_at_both_ends() {
        assert_eq!(splitter().resolve(120.0, -9_000.0), 100.0);
        assert_eq!(splitter().resolve(480.0, 9_000.0), 500.0);
    }

    #[test]
    fn a_clamped_drag_does_not_accumulate_hidden_width() {
        // Reading the width back each frame is what makes this true: pushing
        // far past the minimum and then pulling back must move immediately,
        // not spend the overshoot first.
        let s = splitter();
        let pinned = s.resolve(100.0, -500.0);
        assert_eq!(pinned, 100.0);
        assert_eq!(s.resolve(pinned, 25.0), 125.0);
    }
}
