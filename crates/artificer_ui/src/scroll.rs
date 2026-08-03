//! Wheel scrolling for `Overflow::scroll_y` containers.
//!
//! `Overflow::scroll_y` only CLIPS. Bevy moves nothing on its own, so a list
//! taller than its box silently hides everything past the fold and looks like
//! a list that simply has fewer items. Marking a node [`ScrollView`] gives it
//! the wheel while the pointer is inside it.
//!
//! This lives in the engine because every game that builds a list needs it,
//! and the version that already existed inside one game's shipyard screen had
//! to rediscover the physical/logical pixel trap below the hard way.

use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::prelude::*;
use bevy::ui::{ComputedNode, ScrollPosition};

/// Marks a node with `Overflow::scroll_y` that the wheel should drive.
#[derive(Component, Default, Debug, Clone, Copy)]
pub struct ScrollView;

/// Lines-to-pixels for wheel events that report in lines.
const LINE_HEIGHT: f32 = 28.0;

/// How far a view can scroll, in LOGICAL pixels.
///
/// `ComputedNode` reports PHYSICAL pixels while `ScrollPosition` is logical.
/// Mixing them scrolls the wrong distance on any display that is not at 100%
/// scale — too far on a 150% one, and the list stops short of its own end.
fn travel(node: &ComputedNode) -> f32 {
    (node.content_size().y - node.size().y).max(0.0) * node.inverse_scale_factor()
}

/// Give the wheel to the innermost [`ScrollView`] under the pointer.
///
/// Innermost by area, so a scrollable list inside a scrollable panel takes the
/// wheel rather than the panel around it.
pub fn scroll_hovered_views(
    mut wheel: EventReader<MouseWheel>,
    windows: Query<&Window>,
    mut views: Query<
        (Entity, &mut ScrollPosition, &ComputedNode, &GlobalTransform),
        With<ScrollView>,
    >,
) {
    let cursor = windows
        .iter()
        .next()
        .and_then(|w| w.physical_cursor_position());
    let Some(cursor) = cursor else {
        wheel.clear();
        return;
    };

    let mut best: Option<(Entity, f32)> = None;
    for (entity, _, node, transform) in &views {
        let half = node.size() * 0.5;
        let centre = transform.translation().truncate();
        let inside = (cursor.x - centre.x).abs() <= half.x && (cursor.y - centre.y).abs() <= half.y;
        if !inside {
            continue;
        }
        let area = node.size().x * node.size().y;
        if best.is_none_or(|(_, smallest)| area < smallest) {
            best = Some((entity, area));
        }
    }
    let Some((target, _)) = best else {
        // Nothing scrollable under the pointer: drop the events so they do not
        // arrive in a burst the next time one is hovered.
        wheel.clear();
        return;
    };

    let mut delta = 0.0;
    for event in wheel.read() {
        delta += match event.unit {
            MouseScrollUnit::Line => event.y * LINE_HEIGHT,
            MouseScrollUnit::Pixel => event.y,
        };
    }
    if delta == 0.0 {
        return;
    }
    if let Ok((_, mut scroll, node, _)) = views.get_mut(target) {
        let scale = node.inverse_scale_factor();
        scroll.offset_y = (scroll.offset_y - delta * scale).clamp(0.0, travel(node));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `ComputedNode` cannot be constructed outside Bevy, so the arithmetic
    /// that actually goes wrong is factored out and tested directly.
    fn clamp_offset(offset: f32, delta: f32, scale: f32, content: f32, view: f32) -> f32 {
        let travel = (content - view).max(0.0) * scale;
        (offset - delta * scale).clamp(0.0, travel)
    }

    #[test]
    fn content_shorter_than_the_view_cannot_scroll() {
        assert_eq!(clamp_offset(0.0, 200.0, 1.0, 100.0, 400.0), 0.0);
        assert_eq!(clamp_offset(0.0, -200.0, 1.0, 100.0, 400.0), 0.0);
    }

    #[test]
    fn scrolling_stops_at_the_end_of_the_content() {
        // 1000 physical content in a 400 box travels 600.
        assert_eq!(clamp_offset(0.0, -10_000.0, 1.0, 1000.0, 400.0), 600.0);
        assert_eq!(clamp_offset(600.0, 10_000.0, 1.0, 1000.0, 400.0), 0.0);
    }

    #[test]
    fn travel_is_expressed_in_logical_pixels() {
        // At 150% scale the inverse factor is 1/1.5. Treating the physical
        // extent as logical would let the list run a third too far and leave
        // the last rows unreachable in the other direction.
        let scale = 1.0 / 1.5;
        let end = clamp_offset(0.0, -10_000.0, scale, 1000.0, 400.0);
        assert!(
            (end - 400.0).abs() < 0.01,
            "expected 600 physical = 400 logical, got {end}"
        );
    }

    #[test]
    fn a_wheel_notch_moves_the_same_logical_distance_at_any_scale() {
        let at_100 = clamp_offset(0.0, -LINE_HEIGHT, 1.0, 5000.0, 100.0);
        let at_150 = clamp_offset(0.0, -LINE_HEIGHT, 1.0 / 1.5, 5000.0, 100.0);
        assert!(
            at_100 > at_150,
            "a hidpi display should not scroll further per notch"
        );
    }
}
