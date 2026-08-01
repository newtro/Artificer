//! Panel skins: the same interface, three different materials.
//!
//! A skin is *presentation only*. It changes how a panel is lit, tinted and
//! edged; it never changes layout, hit-testing, or what a panel says. That
//! split is what makes skins swappable at runtime without any screen knowing
//! which one is active.

use bevy::prelude::*;

/// Which skin is active: one of the built-ins, or one a game registered.
///
/// Games ship their own art, and licensed art cannot live in this repository,
/// so a skin has to be describable from outside. `Custom` indexes
/// [`SkinRegistry`], which the game fills at startup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SkinId {
    Builtin(Skin),
    Custom(usize),
}

impl Default for SkinId {
    fn default() -> Self {
        SkinId::Builtin(Skin::default())
    }
}

/// A skin whose frame and body come from textures rather than from maths.
///
/// The three built-ins are procedural because an engine cannot ship art. This
/// is the door for everything else: point it at a nine-sliceable frame and it
/// looks like whatever the art says it looks like.
#[derive(Debug, Clone)]
pub struct TexturedSkin {
    pub name: String,
    /// Frame overlay, drawn on top of the content.
    pub frame: Handle<Image>,
    /// Frame variant used when the panel is selected; falls back to `frame`.
    pub frame_selected: Option<Handle<Image>>,
    /// Body drawn behind the content.
    pub background: Option<Handle<Image>>,
    /// Multiplier on the frame art's own colour.
    ///
    /// Separate from `params.accent` on purpose. White frame art wants
    /// tinting to the accent; art that already carries its own palette wants
    /// near-white here so its colours survive. One field cannot serve both.
    pub frame_tint: Color,
    /// Fraction of the SOURCE texture that is border, per axis (0..0.5).
    /// A 512px frame with a 128px border is 0.25.
    pub source_border: Vec2,
    /// Fraction of the PANEL that the border occupies, per axis. Decoupled
    /// from `source_border` so a wide panel keeps square corners instead of
    /// stretching them.
    pub panel_border: Vec2,
    pub params: SkinParams,
}

/// Skins a game registered at startup.
#[derive(Resource, Debug, Default)]
pub struct SkinRegistry {
    skins: Vec<TexturedSkin>,
}

impl SkinRegistry {
    pub fn register(&mut self, skin: TexturedSkin) -> SkinId {
        self.skins.push(skin);
        SkinId::Custom(self.skins.len() - 1)
    }

    pub fn get(&self, id: SkinId) -> Option<&TexturedSkin> {
        match id {
            SkinId::Custom(i) => self.skins.get(i),
            SkinId::Builtin(_) => None,
        }
    }

    pub fn len(&self) -> usize {
        self.skins.len()
    }

    pub fn is_empty(&self) -> bool {
        self.skins.is_empty()
    }

    /// Name for display, whichever kind of skin it is.
    pub fn name(&self, id: SkinId) -> String {
        match id {
            SkinId::Builtin(s) => s.name().to_string(),
            SkinId::Custom(i) => self
                .skins
                .get(i)
                .map(|s| s.name.clone())
                .unwrap_or_else(|| format!("custom {i}")),
        }
    }

    /// Styling for either kind.
    pub fn params(&self, id: SkinId) -> SkinParams {
        match id {
            SkinId::Builtin(s) => s.params(),
            SkinId::Custom(i) => self
                .skins
                .get(i)
                .map(|s| s.params)
                .unwrap_or_else(|| Skin::default().params()),
        }
    }

    /// Next skin in the cycle, built-ins first then registered ones, so one
    /// key walks every skin the player can actually choose.
    pub fn next(&self, id: SkinId) -> SkinId {
        let order: Vec<SkinId> = Skin::ALL
            .iter()
            .map(|s| SkinId::Builtin(*s))
            .chain((0..self.skins.len()).map(SkinId::Custom))
            .collect();
        let at = order.iter().position(|s| *s == id).unwrap_or(0);
        order[(at + 1) % order.len()]
    }
}

/// Shader branch for a textured skin.
pub const SHADER_MODE_TEXTURED: u32 = 3;

/// Which look a panel wears.
///
/// Ordered as the user picks them in a settings menu, and `Default` is the one
/// a fresh install gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Skin {
    /// Projected light: translucent, emissive, scanlines, fresnel edge glow.
    /// Reads as a hologram; leans on the HDR camera's bloom.
    #[default]
    Holographic,
    /// Hardware: a metal bezel with a screen set into it, shaded edges,
    /// subtle CRT curvature and vignette. Reads as bolted-in equipment.
    Industrial,
    /// Dark glass with a thin accent rule and generous space. Crisp, quiet,
    /// almost no glow.
    Minimal,
}

impl Skin {
    /// Every skin, in menu order.
    pub const ALL: [Skin; 3] = [Skin::Holographic, Skin::Industrial, Skin::Minimal];

    pub fn name(self) -> &'static str {
        match self {
            Skin::Holographic => "Holographic",
            Skin::Industrial => "Industrial",
            Skin::Minimal => "Minimal",
        }
    }

    /// The next skin in the cycle, so a single key can rotate through them.
    pub fn next(self) -> Skin {
        match self {
            Skin::Holographic => Skin::Industrial,
            Skin::Industrial => Skin::Minimal,
            Skin::Minimal => Skin::Holographic,
        }
    }

    /// Shader branch index. Kept beside the WGSL's own `SKIN_*` constants;
    /// they must agree, and the test at the bottom of this file says so.
    pub fn shader_mode(self) -> u32 {
        match self {
            Skin::Holographic => 0,
            Skin::Industrial => 1,
            Skin::Minimal => 2,
        }
    }

    /// The parameters that drive both the shader and the UI content styling.
    pub fn params(self) -> SkinParams {
        match self {
            // Cyan projection, mostly transparent, heavy glow. Alpha comes
            // from content luminance so unlit areas of the panel read as
            // empty air rather than as a dark rectangle.
            Skin::Holographic => SkinParams {
                accent: Color::srgb(0.35, 0.85, 1.0),
                text: Color::srgb(0.80, 0.95, 1.0),
                dim_text: Color::srgb(0.45, 0.65, 0.78),
                backdrop: Color::srgba(0.01, 0.08, 0.13, 0.30),
                emissive: 2.6,
                backdrop_opacity: 0.30,
                scanline_strength: 0.22,
                edge_glow: 1.0,
                flicker: 0.035,
                bezel: 0.0,
                curvature: 0.06,
                corner_radius: 0.030,
                content_inset: Vec2::splat(0.02),
            },
            // Amber-on-dark screen inside a lit metal bezel. Opaque, so it
            // occludes what is behind it like a real object.
            Skin::Industrial => SkinParams {
                accent: Color::srgb(1.0, 0.62, 0.18),
                text: Color::srgb(1.0, 0.83, 0.55),
                dim_text: Color::srgb(0.62, 0.48, 0.32),
                backdrop: Color::srgba(0.05, 0.04, 0.035, 1.0),
                emissive: 1.15,
                backdrop_opacity: 1.0,
                scanline_strength: 0.10,
                edge_glow: 0.18,
                flicker: 0.010,
                bezel: 0.055,
                curvature: 0.14,
                corner_radius: 0.012,
                content_inset: Vec2::new(0.055, 0.075),
            },
            // Dark glass. Almost no treatment: the type does the work.
            Skin::Minimal => SkinParams {
                accent: Color::srgb(0.55, 0.92, 0.85),
                text: Color::srgb(0.93, 0.96, 0.98),
                dim_text: Color::srgb(0.50, 0.56, 0.62),
                backdrop: Color::srgba(0.02, 0.03, 0.04, 0.88),
                emissive: 0.85,
                backdrop_opacity: 0.88,
                scanline_strength: 0.0,
                edge_glow: 0.30,
                flicker: 0.0,
                bezel: 0.0,
                curvature: 0.0,
                corner_radius: 0.018,
                content_inset: Vec2::splat(0.02),
            },
        }
    }
}

/// Tunables shared by the shader and the panel's UI content.
///
/// Colours live here rather than in each screen so that a screen asks for
/// "accent" or "dim text" and gets whatever the active skin means by it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkinParams {
    pub accent: Color,
    pub text: Color,
    pub dim_text: Color,
    pub backdrop: Color,
    /// Multiplier on emitted colour. Above 1 blooms on an HDR camera.
    pub emissive: f32,
    /// How opaque the panel body is where content is dark.
    pub backdrop_opacity: f32,
    pub scanline_strength: f32,
    /// Fresnel rim brightness at grazing angles.
    pub edge_glow: f32,
    /// Amplitude of the brightness flicker, 0 for none.
    pub flicker: f32,
    /// Bezel width as a fraction of panel size; 0 for no bezel.
    pub bezel: f32,
    /// How far the panel bows toward the viewer, in metres per metre.
    pub curvature: f32,
    /// Rounded-corner radius as a fraction of the shorter side.
    pub corner_radius: f32,
    /// Padding applied to the panel's UI root, as a fraction of panel size.
    ///
    /// A skin with a thick frame must push its content inward or the text
    /// slides under the bevel. Procedural skins need almost none; art-driven
    /// ones need whatever their border art occupies.
    pub content_inset: Vec2,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_skin_is_reachable_by_cycling() {
        // A settings key that cycles must visit all of them and come home,
        // or a skin ships that nobody can select.
        let mut seen = vec![Skin::default()];
        let mut at = Skin::default();
        for _ in 0..Skin::ALL.len() {
            at = at.next();
            if !seen.contains(&at) {
                seen.push(at);
            }
        }
        assert_eq!(seen.len(), Skin::ALL.len(), "cycle visits every skin");
        assert_eq!(at, Skin::default(), "and returns to the start");
    }

    #[test]
    fn shader_modes_are_distinct_and_dense() {
        // The WGSL branches on these; a duplicate or a gap silently renders
        // the wrong skin.
        let mut modes: Vec<u32> = Skin::ALL.iter().map(|s| s.shader_mode()).collect();
        modes.sort_unstable();
        assert_eq!(modes, (0..Skin::ALL.len() as u32).collect::<Vec<_>>());
    }

    #[test]
    fn params_are_physically_sane() {
        for skin in Skin::ALL {
            let p = skin.params();
            assert!(p.emissive > 0.0, "{} emits nothing", skin.name());
            assert!(
                (0.0..=1.0).contains(&p.backdrop_opacity),
                "{} opacity out of range",
                skin.name()
            );
            assert!((0.0..=1.0).contains(&p.scanline_strength));
            assert!(p.corner_radius >= 0.0 && p.corner_radius < 0.5);
            assert!(p.curvature >= 0.0);
            // Insets past 40% would leave no room for content at all.
            assert!(
                p.content_inset.x >= 0.0 && p.content_inset.x < 0.4,
                "{} inset x",
                skin.name()
            );
            assert!(p.content_inset.y >= 0.0 && p.content_inset.y < 0.4);
        }
    }
}
