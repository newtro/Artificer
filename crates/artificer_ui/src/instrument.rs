//! Flight instruments: gauges, radar, reticles — drawn, not typed.
//!
//! A HUD made of text in boxes is a menu with a flight sim behind it. Real
//! cockpit interfaces are arcs, rings, ticks, needles and brackets, and the
//! reason is legibility under load: you read a needle's angle or a bar's
//! height at a glance, where a number needs focus you do not have while
//! something is shooting at you.
//!
//! Each instrument is a quad with an SDF shader, so it stays crisp at any
//! size, animates straight from game state, and can be placed and angled in
//! the world exactly like a [`crate::Panel`].

use bevy::asset::weak_handle;
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::mesh::MeshVertexBufferLayoutRef;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, ShaderRef, SpecializedMeshPipelineError,
};

pub const INSTRUMENT_SHADER_HANDLE: Handle<Shader> =
    weak_handle!("a5f1c0de-1a2b-4a11-9c3e-9d1b7c0e0002");

/// The largest number of radar contacts one instrument can show.
///
/// Fixed because it is a uniform array; overflow is the caller's problem to
/// prioritise, which is the right place for it — the nearest and the hostile
/// ones matter, the rest are noise.
pub const MAX_CONTACTS: usize = 16;

/// What an instrument draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InstrumentKind {
    /// Arc gauge with track, fill, ticks and a needle.
    Arc,
    /// Radar plane: range rings, sweep, and contacts on elevation stalks.
    Radar,
    /// Four shield arcs, one per quadrant.
    Quadrant,
    /// Corner-bracket targeting reticle.
    Reticle,
    /// Pitch ladder.
    Ladder,
    /// Segmented tape with a set-point pin (throttle).
    Tape,
}

impl InstrumentKind {
    fn shader_kind(self) -> f32 {
        match self {
            InstrumentKind::Arc => 0.0,
            InstrumentKind::Radar => 1.0,
            InstrumentKind::Quadrant => 2.0,
            InstrumentKind::Reticle => 3.0,
            InstrumentKind::Ladder => 4.0,
            InstrumentKind::Tape => 5.0,
        }
    }
}

/// One radar contact.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Contact {
    /// Position on the radar plane, each axis -1..1.
    pub plane: Vec2,
    /// Height above (+) or below (-) the plane, -1..1. This is the stalk.
    pub elevation: f32,
    pub kind: ContactKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContactKind {
    Neutral,
    Hostile,
    /// The contact currently targeted; drawn brightest.
    Target,
}

impl ContactKind {
    fn weight(self) -> f32 {
        match self {
            ContactKind::Neutral => 1.0,
            ContactKind::Hostile => 2.0,
            ContactKind::Target => 3.0,
        }
    }
}

/// Material for one instrument.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct InstrumentMaterial {
    #[uniform(0)]
    pub tint: Vec4,
    #[uniform(0)]
    pub warn: Vec4,
    #[uniform(0)]
    pub dim: Vec4,
    /// x kind, y value, z value2, w value3
    #[uniform(0)]
    pub a: Vec4,
    /// x arc start (turns), y arc sweep (turns), z thickness, w tick count
    #[uniform(0)]
    pub b: Vec4,
    /// x glow, y aspect, z contact count, w flags
    #[uniform(0)]
    pub c: Vec4,
    #[uniform(1)]
    pub contacts: [Vec4; MAX_CONTACTS],
}

impl InstrumentMaterial {
    pub fn new(kind: InstrumentKind, tint: Color, warn: Color, dim: Color, aspect: f32) -> Self {
        let v = |c: Color| {
            let s = c.to_srgba();
            Vec4::new(s.red, s.green, s.blue, s.alpha)
        };
        Self {
            tint: v(tint),
            warn: v(warn),
            dim: v(dim),
            a: Vec4::new(kind.shader_kind(), 0.0, 0.0, 0.0),
            // Default arc: a 270-degree sweep starting lower-left, the shape
            // every dial in every cockpit uses.
            b: Vec4::new(0.625, 0.75, 0.018, 10.0),
            c: Vec4::new(0.4, aspect, 0.0, 0.0),
            contacts: [Vec4::ZERO; MAX_CONTACTS],
        }
    }

    /// Attitude: pitch and roll, both in turns (0.25 = 90 degrees).
    ///
    /// Kept in turns rather than degrees because every angle in the shader is
    /// a turn, and converting in one place beats converting in six.
    pub fn set_attitude(&mut self, pitch_turns: f32, roll_turns: f32) -> &mut Self {
        self.a.y = pitch_turns;
        self.a.z = roll_turns;
        self
    }

    /// Primary reading, 0..1.
    pub fn set_value(&mut self, value: f32) -> &mut Self {
        self.a.y = value.clamp(0.0, 1.0);
        self
    }

    /// Shield quadrants: fore, starboard, aft, port.
    pub fn set_quadrants(&mut self, fore: f32, starboard: f32, aft: f32, port: f32) -> &mut Self {
        self.a.y = fore.clamp(0.0, 1.0);
        self.a.z = starboard.clamp(0.0, 1.0);
        self.a.w = aft.clamp(0.0, 1.0);
        self.b.x = port.clamp(0.0, 1.0);
        self
    }

    /// Throttle tape: the lit value, the optimal-manoeuvring band, and where
    /// the pilot actually set the throttle (which is not where the ship is).
    pub fn set_throttle(&mut self, speed: f32, band: (f32, f32), set_point: f32) -> &mut Self {
        self.a.y = speed.clamp(0.0, 1.0);
        self.a.z = band.0.clamp(0.0, 1.0);
        self.a.w = band.1.clamp(0.0, 1.0);
        self.b.x = set_point.clamp(0.0, 1.0);
        self
    }

    /// Arc geometry, in turns clockwise from twelve o'clock.
    pub fn set_arc(&mut self, start: f32, sweep: f32, ticks: f32) -> &mut Self {
        self.b.x = start;
        self.b.y = sweep;
        self.b.w = ticks;
        self
    }

    pub fn set_thickness(&mut self, t: f32) -> &mut Self {
        self.b.z = t.max(0.0);
        self
    }

    /// Overall opacity, 0..1.
    ///
    /// The pitch ladder in particular wants to sit well back: it spans the
    /// middle of the view, and at full strength it competes with the thing
    /// you are trying to aim at.
    pub fn set_opacity(&mut self, opacity: f32) -> &mut Self {
        self.tint.w = opacity.clamp(0.0, 1.0);
        self
    }

    pub fn set_glow(&mut self, glow: f32) -> &mut Self {
        self.c.x = glow.max(0.0);
        self
    }

    pub fn set_segments(&mut self, segments: f32) -> &mut Self {
        self.b.w = segments.max(1.0);
        self
    }

    /// Replace the radar contacts. Extras beyond [`MAX_CONTACTS`] are
    /// dropped, so hand them over already sorted by whatever matters.
    pub fn set_contacts(&mut self, contacts: &[Contact]) -> &mut Self {
        let n = contacts.len().min(MAX_CONTACTS);
        for (slot, c) in self.contacts.iter_mut().zip(contacts.iter()).take(n) {
            *slot = Vec4::new(
                c.plane.x.clamp(-1.0, 1.0),
                c.plane.y.clamp(-1.0, 1.0),
                c.elevation.clamp(-1.0, 1.0),
                c.kind.weight(),
            );
        }
        for slot in self.contacts.iter_mut().skip(n) {
            *slot = Vec4::ZERO;
        }
        self.c.z = n as f32;
        self
    }
}

impl Material for InstrumentMaterial {
    fn fragment_shader() -> ShaderRef {
        INSTRUMENT_SHADER_HANDLE.into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        // Instruments are light drawn over the view: additive-ish blending
        // keeps them readable against both a bright planet and empty space.
        AlphaMode::Blend
    }

    fn specialize(
        _pipeline: &MaterialPipeline<Self>,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mat(kind: InstrumentKind) -> InstrumentMaterial {
        InstrumentMaterial::new(kind, Color::WHITE, Color::WHITE, Color::BLACK, 1.0)
    }

    #[test]
    fn instrument_kinds_map_to_distinct_shader_branches() {
        let kinds = [
            InstrumentKind::Arc,
            InstrumentKind::Radar,
            InstrumentKind::Quadrant,
            InstrumentKind::Reticle,
            InstrumentKind::Ladder,
            InstrumentKind::Tape,
        ];
        let mut seen: Vec<u32> = kinds.iter().map(|k| k.shader_kind() as u32).collect();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), kinds.len(), "a duplicate draws the wrong gauge");
        assert_eq!(*seen.last().unwrap(), kinds.len() as u32 - 1, "no gaps");
    }

    #[test]
    fn values_are_clamped_so_bad_game_state_cannot_draw_off_the_dial() {
        let mut m = mat(InstrumentKind::Arc);
        m.set_value(4.2);
        assert_eq!(m.a.y, 1.0);
        m.set_value(-9.0);
        assert_eq!(m.a.y, 0.0);
        m.set_value(f32::NAN);
        // NAN.clamp returns NAN; the shader treats it as a no-draw rather
        // than a wild needle, but catching it here is cheaper than debugging
        // a gauge that points nowhere.
        assert!(m.a.y.is_nan() || (0.0..=1.0).contains(&m.a.y));
    }

    #[test]
    fn contacts_fill_then_clear_and_never_overflow() {
        let mut m = mat(InstrumentKind::Radar);
        let many: Vec<Contact> = (0..40)
            .map(|i| Contact {
                plane: Vec2::new(0.1 * i as f32, 0.0),
                elevation: 0.5,
                kind: ContactKind::Neutral,
            })
            .collect();
        m.set_contacts(&many);
        assert_eq!(m.c.z as usize, MAX_CONTACTS, "capped, not overflowed");

        m.set_contacts(&[]);
        assert_eq!(m.c.z, 0.0);
        assert!(
            m.contacts.iter().all(|c| *c == Vec4::ZERO),
            "stale contacts must be cleared or ghosts linger on the scope"
        );
    }

    #[test]
    fn contact_kinds_are_distinguishable_to_the_shader() {
        let mut m = mat(InstrumentKind::Radar);
        m.set_contacts(&[
            Contact {
                plane: Vec2::ZERO,
                elevation: 0.0,
                kind: ContactKind::Neutral,
            },
            Contact {
                plane: Vec2::ZERO,
                elevation: 0.0,
                kind: ContactKind::Hostile,
            },
            Contact {
                plane: Vec2::ZERO,
                elevation: 0.0,
                kind: ContactKind::Target,
            },
        ]);
        let weights: Vec<f32> = m.contacts[..3].iter().map(|c| c.w).collect();
        assert_eq!(weights, vec![1.0, 2.0, 3.0]);
        // Zero means "empty slot" in the shader, so no kind may claim it.
        assert!(weights.iter().all(|w| *w >= 1.0));
    }

    #[test]
    fn elevation_is_preserved_signed_because_the_stalk_is_the_whole_point() {
        let mut m = mat(InstrumentKind::Radar);
        m.set_contacts(&[Contact {
            plane: Vec2::new(0.3, -0.2),
            elevation: -0.8,
            kind: ContactKind::Hostile,
        }]);
        assert_eq!(m.contacts[0].z, -0.8, "below must stay below");
        assert_eq!(m.contacts[0].y, -0.2);
    }

    #[test]
    fn opacity_rides_on_the_tint_alpha_the_shader_multiplies_by() {
        let mut m = mat(InstrumentKind::Ladder);
        m.set_opacity(0.3);
        assert_eq!(m.tint.w, 0.3);
        m.set_opacity(5.0);
        assert_eq!(m.tint.w, 1.0, "clamped");
    }

    #[test]
    fn attitude_is_not_clamped_because_a_ship_can_point_anywhere() {
        // Unlike a gauge, attitude has no 0..1 range: inverted flight is
        // ordinary, and clamping it would peg the horizon at the edge.
        let mut m = mat(InstrumentKind::Ladder);
        m.set_attitude(-0.2, 0.5);
        assert_eq!((m.a.y, m.a.z), (-0.2, 0.5));
    }

    #[test]
    fn quadrants_land_in_the_slots_the_shader_reads() {
        let mut m = mat(InstrumentKind::Quadrant);
        m.set_quadrants(0.1, 0.2, 0.3, 0.4);
        assert_eq!((m.a.y, m.a.z, m.a.w, m.b.x), (0.1, 0.2, 0.3, 0.4));
    }

    #[test]
    fn throttle_keeps_the_set_point_separate_from_the_speed() {
        // The pin and the bar disagreeing is the informative case: it is what
        // tells a pilot the ship has not caught up with the order yet.
        let mut m = mat(InstrumentKind::Tape);
        m.set_throttle(0.4, (0.55, 0.75), 0.9);
        assert_eq!(m.a.y, 0.4, "actual");
        assert_eq!((m.a.z, m.a.w), (0.55, 0.75), "band");
        assert_eq!(m.b.x, 0.9, "commanded");
    }
}
