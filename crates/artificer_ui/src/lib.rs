//! World-space UI panels: menus and HUD as lit geometry in the 3D scene.
//!
//! The problem this solves is that a 3D game whose menus are flat text drawn
//! over the viewport looks like a terminal with a render behind it. The fix is
//! the one Unreal calls a Widget Component: lay the interface out normally,
//! render it to a texture, and wear that texture on a mesh that lives in the
//! world — lit, post-processed, occluded, and pickable like anything else.
//!
//! ```text
//!   UI tree  ->  panel camera  ->  render target  ->  material on a mesh
//!   (layout)     (2D, isolated)    (an Image)         (in the 3D world)
//! ```
//!
//! What this crate gives you:
//!
//! - [`PanelDesc`] / [`spawn_panel`]: a curved, rounded, world-space quad with
//!   its own isolated UI camera. Put your widgets under the returned
//!   [`PanelHandle::ui_root`] and they appear on the surface.
//! - [`Skin`]: three complete looks — holographic, industrial, minimal —
//!   swappable at runtime without rebuilding anything, because a skin is a
//!   uniform write.
//! - [`PanelRaycast`]: pointer hits in panel-local UV, so a screen can react
//!   to a click on a surface at any angle.
//!
//! It is deliberately game-agnostic: no screens, no content, no art. Games
//! build their cockpit, market and shipyard on top of it (engine ADR-0002
//! assigns those to the game repository).

use bevy::asset::load_internal_asset;
use bevy::pbr::{MaterialPlugin, NotShadowCaster, NotShadowReceiver};
use bevy::prelude::*;
use bevy::render::camera::RenderTarget;
use bevy::render::mesh::{Indices, PrimitiveTopology};
use bevy::render::render_asset::RenderAssetUsages;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};
use bevy::render::view::RenderLayers;

mod instrument;
mod material;
mod skin;

pub use instrument::{
    Contact, ContactKind, InstrumentKind, InstrumentMaterial, INSTRUMENT_SHADER_HANDLE,
    MAX_CONTACTS,
};
pub use material::{PanelMaterial, PANEL_SHADER_HANDLE};
pub use skin::{Skin, SkinId, SkinParams, SkinRegistry, TexturedSkin, SHADER_MODE_TEXTURED};

/// Render layer that panel UI cameras draw, kept away from the world camera.
///
/// Panel content is 2D UI rendered offscreen; if it shared layer 0 it would
/// also be composited over the main view.
const PANEL_UI_LAYER: usize = 31;

/// How many texture pixels one metre of panel is worth by default.
///
/// 512 keeps text crisp at conversational distance without making every panel
/// a megabyte of VRAM. Override per panel for something you press your face
/// against.
pub const DEFAULT_PIXELS_PER_METRE: f32 = 512.0;

/// The active skin. Change this resource and every panel restyles next frame.
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ActiveSkin(pub SkinId);

/// A 1x1 transparent image, bound wherever a skin has no texture.
///
/// Texture bindings are not optional in a bind group, so the procedural skins
/// still need something to point at.
#[derive(Resource, Debug, Clone)]
pub struct BlankTexture(pub Handle<Image>);

/// Marks a panel's root entity, and remembers what it needs to restyle.
#[derive(Component, Debug, Clone)]
pub struct Panel {
    pub size: Vec2,
    /// Entity holding the panel's isolated UI camera.
    pub ui_camera: Entity,
    /// Parent your widgets to this.
    pub ui_root: Entity,
    /// Skin currently baked into the material, for change detection.
    applied: Option<SkinId>,
    /// Selection state baked in, so a textured skin re-binds its frame.
    applied_selected: bool,
    /// Highlighted state, skin-independent.
    pub selected: bool,
    /// 0..1 fade, for show/hide transitions.
    pub opacity: f32,
}

/// What [`spawn_panel`] gives back.
#[derive(Debug, Clone)]
pub struct PanelHandle {
    /// The mesh in the world. Transform this to move the panel.
    pub entity: Entity,
    /// Parent widgets here.
    pub ui_root: Entity,
    pub ui_camera: Entity,
    /// The texture the UI renders into, if you want it elsewhere too.
    pub content: Handle<Image>,
}

/// How a panel should look and how big it is in the world.
#[derive(Debug, Clone)]
pub struct PanelDesc {
    /// Size in metres.
    pub size: Vec2,
    /// Texture resolution; `None` derives it from [`DEFAULT_PIXELS_PER_METRE`].
    pub resolution: Option<UVec2>,
    /// Mesh subdivisions across the width. More = smoother curve.
    pub segments: u32,
    /// Bow toward the viewer, metres of depth at the edges. `None` takes the
    /// skin's own curvature, which is usually what you want.
    pub curvature: Option<f32>,
}

impl Default for PanelDesc {
    fn default() -> Self {
        Self {
            size: Vec2::new(1.6, 0.9),
            resolution: None,
            segments: 24,
            curvature: None,
        }
    }
}

impl PanelDesc {
    pub fn size(mut self, width: f32, height: f32) -> Self {
        self.size = Vec2::new(width, height);
        self
    }

    pub fn resolution(mut self, width: u32, height: u32) -> Self {
        self.resolution = Some(UVec2::new(width, height));
        self
    }

    pub fn curvature(mut self, depth: f32) -> Self {
        self.curvature = Some(depth);
        self
    }

    fn pixels(&self) -> UVec2 {
        self.resolution.unwrap_or_else(|| {
            UVec2::new(
                (self.size.x * DEFAULT_PIXELS_PER_METRE).round().max(16.0) as u32,
                (self.size.y * DEFAULT_PIXELS_PER_METRE).round().max(16.0) as u32,
            )
        })
    }
}

/// Register the panel material, shader and restyle system.
pub struct ArtificerUiPlugin;

impl Plugin for ArtificerUiPlugin {
    fn build(&self, app: &mut App) {
        load_internal_asset!(
            app,
            PANEL_SHADER_HANDLE,
            "shaders/panel.wgsl",
            Shader::from_wgsl
        );
        load_internal_asset!(
            app,
            INSTRUMENT_SHADER_HANDLE,
            "shaders/instrument.wgsl",
            Shader::from_wgsl
        );
        app.init_resource::<ActiveSkin>()
            .init_resource::<SkinRegistry>()
            .add_plugins(MaterialPlugin::<PanelMaterial>::default())
            .add_plugins(MaterialPlugin::<InstrumentMaterial>::default())
            .add_systems(PreStartup, create_blank_texture)
            .add_systems(Update, restyle_panels);
    }
}

fn create_blank_texture(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let image = Image::new_fill(
        Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &[0, 0, 0, 0],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    commands.insert_resource(BlankTexture(images.add(image)));
}

/// Push the active skin (and per-panel state) into panel materials.
///
/// Runs every frame but writes only on change, so an idle scene of panels
/// costs a comparison each.
#[allow(clippy::type_complexity)]
fn restyle_panels(
    active: Res<ActiveSkin>,
    registry: Res<SkinRegistry>,
    mut materials: ResMut<Assets<PanelMaterial>>,
    mut panels: Query<(&mut Panel, &MeshMaterial3d<PanelMaterial>)>,
    mut roots: Query<&mut Node>,
) {
    for (mut panel, handle) in &mut panels {
        let Some(material) = materials.get_mut(&handle.0) else {
            continue;
        };
        let aspect = panel.size.x / panel.size.y.max(0.0001);
        let selected = panel.selected;
        let opacity = panel.opacity;
        // Textured skins also re-bind on a selection change, because the art
        // supplies a whole separate frame for the selected state.
        let restyle = panel.applied != Some(active.0) || panel.applied_selected != selected;
        if restyle {
            match active.0 {
                SkinId::Builtin(s) => material.apply(s, &s.params(), aspect),
                SkinId::Custom(_) => match registry.get(active.0) {
                    Some(t) => material.apply_textured(t, aspect, selected),
                    None => {
                        // Registered skin vanished (hot reload, bad index):
                        // fall back rather than render an untextured mess.
                        let s = Skin::default();
                        material.apply(s, &s.params(), aspect);
                    }
                },
            }
            // Push the content in far enough to clear the frame art.
            let inset = registry.params(active.0).content_inset;
            if let Ok(mut node) = roots.get_mut(panel.ui_root) {
                node.padding =
                    UiRect::axes(Val::Percent(inset.x * 100.0), Val::Percent(inset.y * 100.0));
            }
            panel.applied = Some(active.0);
            panel.applied_selected = selected;
        }
        material.set_selected(selected);
        material.set_opacity(opacity);
    }
}

/// A flat quad for an instrument, sized in metres.
///
/// Instruments are drawn entirely by their shader, so unlike a panel they
/// need no render target, no camera and no UI tree — just geometry to draw on.
pub fn instrument_quad(meshes: &mut Assets<Mesh>, size: Vec2) -> Handle<Mesh> {
    meshes.add(curved_quad(size, 1, 0.0))
}

/// Build a panel: mesh, material, render target, and an isolated UI camera.
///
/// The returned `ui_root` is an ordinary UI node — lay widgets out under it
/// with the normal flexbox API and they land on the surface.
#[allow(clippy::too_many_arguments)]
pub fn spawn_panel(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<PanelMaterial>,
    images: &mut Assets<Image>,
    desc: &PanelDesc,
    skin: SkinId,
    blank: Handle<Image>,
    transform: Transform,
) -> PanelHandle {
    let pixels = desc.pixels();

    // The render target. `RENDER_ATTACHMENT` lets a camera draw into it;
    // `TEXTURE_BINDING` lets the panel material sample it.
    let mut image = Image::new_fill(
        Extent3d {
            width: pixels.x.max(1),
            height: pixels.y.max(1),
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &[0, 0, 0, 0],
        // Rgba8UnormSrgb so text authored in sRGB reads back at the same
        // brightness the layout intended.
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    image.texture_descriptor.usage =
        TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST | TextureUsages::RENDER_ATTACHMENT;
    let content = images.add(image);

    // The panel's own UI camera, on its own render layer, clearing to
    // transparent so unpainted areas of the panel really are empty.
    let ui_camera = commands
        .spawn((
            Camera2d,
            Camera {
                target: RenderTarget::Image(content.clone().into()),
                clear_color: ClearColorConfig::Custom(Color::NONE),
                // Panels render before the world camera composites them.
                order: -1,
                ..default()
            },
            RenderLayers::layer(PANEL_UI_LAYER),
        ))
        .id();

    // Widgets go here. Full-bleed so the layout's coordinate space is the
    // panel's surface.
    let ui_root = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                ..default()
            },
            UiTargetCamera(ui_camera),
            RenderLayers::layer(PANEL_UI_LAYER),
        ))
        .id();

    // Curvature comes from the built-in skin when the caller does not say;
    // a textured skin's art already implies its own depth, so it stays flat
    // unless asked otherwise.
    let builtin = match skin {
        SkinId::Builtin(s) => Some(s),
        SkinId::Custom(_) => None,
    };
    let curvature = desc.curvature.unwrap_or_else(|| {
        builtin
            .map(|s| s.params().curvature * desc.size.x)
            .unwrap_or(0.0)
    });
    let mesh = meshes.add(curved_quad(desc.size, desc.segments.max(1), curvature));
    let material = materials.add(PanelMaterial::new(
        builtin.unwrap_or_default(),
        content.clone(),
        blank,
        desc.size.x / desc.size.y.max(0.0001),
    ));

    let entity = commands
        .spawn((
            Mesh3d(mesh),
            MeshMaterial3d(material),
            transform,
            // A UI surface neither casts nor catches shadows; it is light.
            NotShadowCaster,
            NotShadowReceiver,
            Panel {
                size: desc.size,
                ui_camera,
                ui_root,
                // Left unset so the first restyle binds a textured skin's
                // art; `new` only baked the procedural defaults.
                applied: builtin.map(SkinId::Builtin),
                applied_selected: false,
                selected: false,
                opacity: 1.0,
            },
        ))
        .id();

    PanelHandle {
        entity,
        ui_root,
        ui_camera,
        content,
    }
}

/// A quad in the XY plane, subdivided across X and bowed along +Z.
///
/// Curvature is what stops a panel reading as a decal: a slight bow catches
/// the rim light differently along its width and gives the surface somewhere
/// for a specular streak to travel.
fn curved_quad(size: Vec2, segments: u32, depth: f32) -> Mesh {
    let cols = segments.max(1);
    let half = size * 0.5;
    let mut positions: Vec<[f32; 3]> = Vec::with_capacity(((cols + 1) * 2) as usize);
    let mut normals: Vec<[f32; 3]> = Vec::with_capacity(((cols + 1) * 2) as usize);
    let mut uvs: Vec<[f32; 2]> = Vec::with_capacity(((cols + 1) * 2) as usize);
    let mut indices: Vec<u32> = Vec::with_capacity((cols * 6) as usize);

    for i in 0..=cols {
        let t = i as f32 / cols as f32;
        let x = -half.x + size.x * t;
        // Parabolic bow: flat in the middle, deepest at the edges.
        let k = (t - 0.5) * 2.0;
        let z = -depth * (1.0 - k * k);
        // Normal follows the surface slope so the fresnel rim behaves.
        let slope = if depth.abs() > f32::EPSILON {
            2.0 * depth * k * 2.0 / size.x.max(0.0001)
        } else {
            0.0
        };
        let n = Vec3::new(slope, 0.0, 1.0).normalize();

        for (row, y) in [(0u32, -half.y), (1, half.y)] {
            positions.push([x, y, z]);
            normals.push([n.x, n.y, n.z]);
            // v flipped: UI y grows downward, texture v grows upward.
            uvs.push([t, 1.0 - row as f32]);
        }
    }
    for i in 0..cols {
        let bl = i * 2;
        let tl = bl + 1;
        let br = bl + 2;
        let tr = bl + 3;
        indices.extend_from_slice(&[bl, br, tr, bl, tr, tl]);
    }

    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_indices(Indices::U32(indices))
}

/// Where a ray met a panel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PanelRaycast {
    pub entity: Entity,
    /// Hit point in panel UV: (0,0) top-left of the surface, (1,1) bottom-right.
    /// Feed straight to hit-testing against the widget layout.
    pub uv: Vec2,
    /// Distance along the ray, for picking the nearest of several panels.
    pub distance: f32,
}

/// Intersect a world-space ray with a panel's plane and return the UV hit.
///
/// Treats the panel as flat even when it is bowed: the curve is at most a few
/// centimetres and a pointer that lands a pixel or two off on a curved surface
/// is not something anyone can perceive, whereas a curved-surface solve is a
/// Newton iteration nobody needs.
pub fn raycast_panel(
    panel: &Panel,
    panel_transform: &GlobalTransform,
    entity: Entity,
    ray_origin: Vec3,
    ray_dir: Vec3,
) -> Option<PanelRaycast> {
    let to_local = panel_transform.affine().inverse();
    let local_origin = to_local.transform_point3(ray_origin);
    let local_dir = to_local.transform_vector3(ray_dir);
    if local_dir.z.abs() < 1e-6 {
        return None; // edge-on: no meaningful hit
    }
    let t = -local_origin.z / local_dir.z;
    if t < 0.0 {
        return None; // behind the pointer
    }
    let hit = local_origin + local_dir * t;
    let half = panel.size * 0.5;
    if hit.x < -half.x || hit.x > half.x || hit.y < -half.y || hit.y > half.y {
        return None;
    }
    Some(PanelRaycast {
        entity,
        uv: Vec2::new(
            (hit.x + half.x) / panel.size.x,
            // UI space runs downward from the top edge.
            1.0 - (hit.y + half.y) / panel.size.y,
        ),
        distance: t * local_dir.length(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_panel(size: Vec2) -> Panel {
        Panel {
            size,
            ui_camera: Entity::PLACEHOLDER,
            ui_root: Entity::PLACEHOLDER,
            applied: None,
            applied_selected: false,
            selected: false,
            opacity: 1.0,
        }
    }

    #[test]
    fn a_flat_panel_is_two_triangles_with_full_uv_coverage() {
        let mesh = curved_quad(Vec2::new(2.0, 1.0), 1, 0.0);
        assert_eq!(mesh.count_vertices(), 4);
        let uvs = mesh
            .attribute(Mesh::ATTRIBUTE_UV_0)
            .and_then(|a| match a {
                bevy::render::mesh::VertexAttributeValues::Float32x2(v) => Some(v.clone()),
                _ => None,
            })
            .expect("uv0");
        // Corners must span the whole texture or the content is cropped.
        assert!(uvs.contains(&[0.0, 0.0]) && uvs.contains(&[1.0, 1.0]));
    }

    #[test]
    fn curvature_bows_the_edges_toward_the_viewer_and_leaves_the_centre() {
        let mesh = curved_quad(Vec2::new(2.0, 1.0), 4, 0.1);
        let pos = mesh
            .attribute(Mesh::ATTRIBUTE_POSITION)
            .and_then(|a| match a {
                bevy::render::mesh::VertexAttributeValues::Float32x3(v) => Some(v.clone()),
                _ => None,
            })
            .expect("positions");
        let centre_z = pos.iter().find(|p| p[0].abs() < 1e-6).expect("centre")[2];
        let edge_z = pos.iter().find(|p| p[0] <= -0.999).expect("left edge")[2];
        assert!(
            centre_z < edge_z,
            "centre {centre_z} should sit deeper than edge {edge_z}"
        );
        assert!((edge_z - 0.0).abs() < 1e-5, "edges stay on the plane");
    }

    #[test]
    fn a_ray_down_the_middle_hits_the_centre_of_the_surface() {
        let panel = test_panel(Vec2::new(2.0, 1.0));
        let xform = GlobalTransform::from(Transform::IDENTITY);
        let hit = raycast_panel(
            &panel,
            &xform,
            Entity::PLACEHOLDER,
            Vec3::new(0.0, 0.0, 5.0),
            Vec3::new(0.0, 0.0, -1.0),
        )
        .expect("straight-on ray hits");
        assert!(
            (hit.uv - Vec2::new(0.5, 0.5)).length() < 1e-5,
            "{:?}",
            hit.uv
        );
        assert!((hit.distance - 5.0).abs() < 1e-4);
    }

    #[test]
    fn uv_origin_is_the_top_left_the_way_ui_layout_expects() {
        // A hit above centre must read as a SMALL v, because UI y grows down.
        // Getting this backwards flips every screen's hit-testing.
        let panel = test_panel(Vec2::new(2.0, 1.0));
        let xform = GlobalTransform::from(Transform::IDENTITY);
        let upper_left = raycast_panel(
            &panel,
            &xform,
            Entity::PLACEHOLDER,
            Vec3::new(-0.9, 0.4, 5.0),
            Vec3::new(0.0, 0.0, -1.0),
        )
        .expect("hit");
        assert!(upper_left.uv.x < 0.1, "left edge -> u near 0");
        assert!(upper_left.uv.y < 0.2, "above centre -> v near 0");
    }

    #[test]
    fn rays_that_miss_the_surface_report_nothing() {
        let panel = test_panel(Vec2::new(2.0, 1.0));
        let xform = GlobalTransform::from(Transform::IDENTITY);
        // Past the edge.
        assert!(raycast_panel(
            &panel,
            &xform,
            Entity::PLACEHOLDER,
            Vec3::new(2.0, 0.0, 5.0),
            Vec3::NEG_Z,
        )
        .is_none());
        // Pointing away.
        assert!(raycast_panel(
            &panel,
            &xform,
            Entity::PLACEHOLDER,
            Vec3::new(0.0, 0.0, 5.0),
            Vec3::Z,
        )
        .is_none());
        // Edge-on.
        assert!(raycast_panel(
            &panel,
            &xform,
            Entity::PLACEHOLDER,
            Vec3::new(0.0, 0.0, 5.0),
            Vec3::X,
        )
        .is_none());
    }

    #[test]
    fn a_rotated_panel_still_maps_hits_correctly() {
        // Panels are placed at angles around the player; picking must follow
        // the transform rather than assuming a screen-aligned plane.
        let panel = test_panel(Vec2::new(2.0, 1.0));
        let xform = GlobalTransform::from(
            Transform::from_xyz(3.0, 0.0, 0.0)
                .with_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2)),
        );
        // Panel now faces +X, so shoot along -X from further out.
        let hit = raycast_panel(
            &panel,
            &xform,
            Entity::PLACEHOLDER,
            Vec3::new(9.0, 0.0, 0.0),
            Vec3::NEG_X,
        )
        .expect("rotated panel is hit");
        assert!(
            (hit.uv - Vec2::new(0.5, 0.5)).length() < 1e-4,
            "{:?}",
            hit.uv
        );
    }
}
