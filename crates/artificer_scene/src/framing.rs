//! Framing a camera's subject into a UI-defined region of the window.
//!
//! A game that draws 2D chrome around a 3D scene rarely wants the subject in
//! the middle of the WINDOW -- it wants it in the middle of the hole the
//! chrome leaves. The first game to need this hardcoded the pan as a constant
//! derived from one layout's panel widths; the layout changed and the subject
//! left the screen with nothing to say why. The rule this module encodes: a
//! number that describes a layout has to come FROM the layout, and the maths
//! that consumes it belongs to the engine, not to every game separately.

use glam::Vec3;

/// A rectangle of the window, in window fractions, that a camera should frame
/// its subject into. Published by whoever owns the layout.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewportRect {
    /// Centre of the region, 0..1 from the window's top-left.
    pub cx: f32,
    pub cy: f32,
    /// Size of the region, as a fraction of the window.
    pub w: f32,
    pub h: f32,
    /// Window width over height. The vertical field of view is fixed by the
    /// camera; the horizontal one is whatever the window shape makes it, so
    /// horizontal fractions cannot become world units without this.
    pub aspect: f32,
}

impl Default for ViewportRect {
    /// The whole window, centred: what framing means before any layout has
    /// been measured -- and on the first frame, none has.
    fn default() -> Self {
        Self {
            cx: 0.5,
            cy: 0.5,
            w: 1.0,
            h: 1.0,
            aspect: 16.0 / 9.0,
        }
    }
}

/// Smallest region the maths will frame into.
///
/// A degenerate or not-yet-measured rect would otherwise divide the camera
/// distance by nearly zero and fling the eye out past the far clip.
const MIN_VIEWPORT_FRACTION: f32 = 0.15;

/// Adjust a camera rig so its subject fills `rect` instead of the window.
///
/// `eye` and `focus` describe the FULL-WINDOW framing the caller wants --
/// focus on the subject, eye wherever its orbit puts it. The result backs the
/// eye off so the subject fits the region's smaller axis, then PANS the whole
/// rig -- eye and focus together -- so the subject lands at the region's
/// centre. A pan, not a rotation, deliberately: the subject must not be
/// skewed or seen from an angle it was not framed for.
///
/// The BINDING constraint is the smaller fraction: a wide, short hole crops
/// the subject top and bottom long before it runs out of width, so fitting to
/// width alone still cuts it in half.
pub fn fit_into(eye: Vec3, focus: Vec3, rect: ViewportRect, fov_y_degrees: f32) -> (Vec3, Vec3) {
    let fit = rect.w.min(rect.h).clamp(MIN_VIEWPORT_FRACTION, 1.0);
    let offset = eye - focus;
    let distance = offset.length() / fit;
    let eye = focus + offset.normalize_or_zero() * distance;

    // Camera basis, from the look direction. `forward x up` is +X when
    // forward is -Z, which is the handedness these scenes use.
    let forward = (focus - eye).normalize_or_zero();
    let right = forward.cross(Vec3::Y).normalize_or_zero();
    let up = right.cross(forward).normalize_or_zero();

    // Where the region sits in normalised device coords: +1 is the right or
    // top edge of the window. Screen Y runs DOWN and world up runs up, so the
    // vertical term is inverted.
    let ndc_x = (rect.cx - 0.5) * 2.0;
    let ndc_y = (0.5 - rect.cy) * 2.0;

    // Half-extents of what the camera sees at the subject's distance.
    let half_height = distance * (fov_y_degrees.to_radians() * 0.5).tan();
    let half_width = half_height * rect.aspect;

    // Moving the camera by +v moves the IMAGE by -v, so the pan is negated:
    // to put the subject right of centre, the rig goes left.
    let pan = -right * (ndc_x * half_width) - up * (ndc_y * half_height);
    (eye + pan, focus + pan)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rig() -> (Vec3, Vec3) {
        let focus = Vec3::new(10.0, 2.0, -5.0);
        let eye = focus + Vec3::new(0.0, 6.0, 18.0);
        (eye, focus)
    }

    /// A full-window rect must not move anything: framing into the whole
    /// window IS the caller's own framing. The bug this pins: a pan baked in
    /// as a constant stayed applied when the layout stopped needing it.
    #[test]
    fn a_full_window_rect_is_the_identity() {
        let (eye, focus) = rig();
        let (fitted_eye, fitted_focus) = fit_into(eye, focus, ViewportRect::default(), 60.0);
        assert!(fitted_eye.distance(eye) < 1.0e-4);
        assert!(fitted_focus.distance(focus) < 1.0e-4);
    }

    /// A rect left of the window centre pans the rig RIGHT, because moving
    /// the camera one way moves the image the other.
    #[test]
    fn an_off_centre_rect_pans_the_rig_the_other_way() {
        let (eye, focus) = rig();
        let rect = ViewportRect {
            cx: 0.39,
            cy: 0.57,
            w: 0.66,
            h: 0.65,
            aspect: 16.0 / 9.0,
        };
        let (fitted_eye, fitted_focus) = fit_into(eye, focus, rect, 60.0);
        assert!(
            fitted_focus.distance(focus) > 0.1,
            "an off-centre rect must pan"
        );
        let forward = (fitted_focus - fitted_eye).normalize_or_zero();
        let right = forward.cross(Vec3::Y).normalize_or_zero();
        assert!(
            (fitted_focus - focus).dot(right) > 0.0,
            "a rect left of centre should pan the rig right"
        );
    }

    /// A smaller hole sits the camera further back, or the subject simply
    /// overflows it.
    #[test]
    fn a_smaller_rect_backs_the_camera_off() {
        let (eye, focus) = rig();
        let small = ViewportRect {
            w: 0.4,
            h: 0.4,
            ..ViewportRect::default()
        };
        let (far_eye, _) = fit_into(eye, focus, small, 60.0);
        assert!(
            far_eye.distance(focus) > eye.distance(focus) + 1.0,
            "a 40% hole must push the camera back"
        );
    }

    /// The clamp holds for degenerate rects, so a layout that has not run yet
    /// cannot fling the eye past the far clip.
    #[test]
    fn a_degenerate_rect_cannot_explode_the_distance() {
        let (eye, focus) = rig();
        let broken = ViewportRect {
            w: 0.0,
            h: 0.0,
            ..ViewportRect::default()
        };
        let (fitted_eye, _) = fit_into(eye, focus, broken, 60.0);
        let base = eye.distance(focus);
        assert!(
            fitted_eye.distance(focus) <= base / MIN_VIEWPORT_FRACTION + 1.0e-3,
            "the minimum fraction must bound the back-off"
        );
    }
}
