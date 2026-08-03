//! Point-in-node hit-testing for `bevy_ui` trees, in PHYSICAL pixels.
//!
//! Bevy gives interactive nodes `Interaction`, but a game that hit-tests its
//! own gestures -- drops, wheel routing, pointer capture -- has to compare a
//! cursor against node bounds itself. Every game that did so re-derived the
//! same three facts, and one of them twice with a bug:
//!
//! - `ComputedNode::size()` and the UI `GlobalTransform` are PHYSICAL pixels,
//!   so testing them against the logical cursor misclassifies on any display
//!   that is not at 100% scale;
//! - a clipped node still HAS bounds -- a row scrolled out of a list can sit
//!   under the cursor invisibly, and taking a drop there fits a component to
//!   a socket the player cannot see;
//! - when nested nodes overlap, the SMALLEST container is the specific
//!   target.
//!
//! These helpers take `physical_cursor_position()`, never `cursor_position()`.

use bevy::prelude::*;
use bevy::ui::ComputedNode;

/// Whether a physical-pixel point is inside a node's bounds.
pub fn node_contains(node: &ComputedNode, transform: &GlobalTransform, point: Vec2) -> bool {
    let half = node.size() * 0.5;
    let centre = transform.translation().truncate();
    (point.x - centre.x).abs() <= half.x && (point.y - centre.y).abs() <= half.y
}

/// Whether a point on a node is actually VISIBLE, or clipped away by an
/// ancestor (a scroll container, usually). A node that is not drawn must not
/// take a drop.
pub fn point_visible(clip: Option<&CalculatedClip>, point: Vec2) -> bool {
    clip.is_none_or(|clip| clip.clip.contains(point))
}

/// Whether a physical-pixel point is inside a node AND visible.
pub fn hit(
    node: &ComputedNode,
    transform: &GlobalTransform,
    clip: Option<&CalculatedClip>,
    point: Vec2,
) -> bool {
    node_contains(node, transform, point) && point_visible(clip, point)
}

/// The on-screen area of a node, for smallest-wins target selection between
/// overlapping candidates: a nested control inside a row is a more specific
/// target than the row itself.
pub fn area(node: &ComputedNode) -> f32 {
    node.size().x * node.size().y
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `CalculatedClip` cannot be constructed freely across versions, so the
    /// pure containment maths is what these cover; clip behaviour is a thin
    /// `Rect::contains` delegation.
    #[test]
    fn no_clip_means_visible() {
        assert!(point_visible(None, Vec2::new(5.0, 5.0)));
    }
}
