//! A live, orbitable 3D view of one asset, rendered to an image.
//!
//! [`crate::thumbnail`] answers "what does this look like" with a still that
//! tears itself down; this answers "let me look at it" and stays up. An asset
//! browser needs both: a still per grid tile, and one turntable for whatever
//! is selected.
//!
//! Structurally it is the same trick — an isolated render layer drawn to a
//! `Handle<Image>` you can hang in an `ImageNode` — but the camera persists and
//! is driven by the pointer, and the content swaps as the selection changes.

use bevy::asset::RenderAssetUsages;
use bevy::image::Image;
use bevy::prelude::*;
use bevy::render::camera::{ClearColorConfig, RenderTarget};
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};
use bevy::render::view::RenderLayers;

/// Layer the preview stage draws on.
///
/// Above the thumbnail pool (16..=23) and below [`crate::HUD_LAYER`] (30) and
/// the panel UI layer (31), so a preview never shares a pass with an icon
/// capture or with live UI.
pub const PREVIEW_LAYER: usize = 24;

/// Where the orbit camera is looking from.
///
/// Angles rather than a transform because the pointer drives angles: a drag is
/// a yaw/pitch delta, and storing the matrix would mean decomposing it back
/// every frame to clamp the pitch.
#[derive(Debug, Clone, Copy)]
pub struct Orbit {
    pub yaw: f32,
    pub pitch: f32,
    /// Distance as a multiple of the asset's bounding radius, so the framing
    /// is identical for a 40 m artifact and a 200 m megastructure.
    pub distance: f32,
}

impl Default for Orbit {
    fn default() -> Self {
        // Three-quarter view from slightly above: a square-on render of most
        // hard-surface art reads as an ambiguous rectangle.
        Self {
            yaw: 0.6,
            pitch: 0.42,
            // A multiple of the fit distance: just over 1.0 so the model fills
            // the viewport with a little air rather than grazing the edges.
            distance: 1.12,
        }
    }
}

/// Vertical field of view of the preview camera.
///
/// Bevy's `PerspectiveProjection` default, restated here because the framing
/// maths needs it and reading it back off the camera every frame just to
/// recompute a constant would be silly.
const PREVIEW_FOV_Y: f32 = std::f32::consts::FRAC_PI_4;

/// Distance at which a bounding sphere of `radius` exactly fills the frame.
///
/// Depends on ASPECT, not just fov: a viewport narrower than it is tall runs
/// out of horizontal room first, and framing on the vertical angle alone would
/// let the model spill out of the sides the moment the panel is dragged
/// narrow. Fitting on whichever half-angle is tighter is what makes the model
/// keep filling the space at any panel size the user drags to.
pub fn fit_distance(radius: f32, fov_y: f32, aspect: f32) -> f32 {
    let half_v = (fov_y * 0.5).clamp(0.01, 1.5);
    // Horizontal half-angle implied by this aspect ratio.
    let half_h = (aspect.max(0.01) * half_v.tan()).atan();
    let tighter = half_v.min(half_h);
    radius / tighter.sin().max(1e-4)
}

impl Orbit {
    /// Just short of the poles. Exactly vertical makes `looking_at` degenerate
    /// -- the up vector and the view direction become parallel and the image
    /// snaps to a random roll.
    const PITCH_LIMIT: f32 = 1.45;
    /// `distance` is a multiple of the FIT distance, so 1.0 means "exactly
    /// filling" and the bounds are how far in and out of that the wheel goes.
    const MIN_DISTANCE: f32 = 0.45;
    const MAX_DISTANCE: f32 = 4.0;

    pub fn turn(&mut self, delta: Vec2) {
        self.yaw -= delta.x;
        self.pitch = (self.pitch + delta.y).clamp(-Self::PITCH_LIMIT, Self::PITCH_LIMIT);
    }

    /// Multiplicative zoom, so one wheel notch covers the same visual step
    /// whether you are close in or far out.
    pub fn zoom(&mut self, notches: f32) {
        self.distance =
            (self.distance * (1.0 - notches * 0.12)).clamp(Self::MIN_DISTANCE, Self::MAX_DISTANCE);
    }

    /// Camera position for a model of `radius` in a viewport of `aspect`.
    fn eye(&self, radius: f32, aspect: f32) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        let fit = fit_distance(radius, PREVIEW_FOV_Y, aspect);
        Vec3::new(cp * sy, sp, cp * cy) * fit * self.distance
    }
}

/// Marks the persistent preview camera.
#[derive(Component)]
struct PreviewCamera;

/// Marks the UI node that displays [`PreviewStage::target`].
///
/// Tagging the node is all a game has to do: the stage then follows its size,
/// so a resizable panel keeps a correctly-shaped render target and a model
/// that still fills it, with no per-game plumbing.
#[derive(Component, Default, Debug, Clone, Copy)]
pub struct PreviewViewportNode;

/// Track the size of the node showing the preview.
fn follow_viewport_node(
    mut stage: ResMut<PreviewStage>,
    nodes: Query<&bevy::ui::ComputedNode, With<PreviewViewportNode>>,
) {
    let Some(node) = nodes.iter().next() else {
        return;
    };
    // PHYSICAL pixels: the render target is a texture, so it wants the real
    // pixel count, not the layout's logical one. On a 150% display a logical
    // size would render a texture two thirds the resolution of the box it is
    // stretched across.
    let size = node.size();
    if size.x > 0.0 && size.y > 0.0 {
        stage.set_viewport(UVec2::new(size.x as u32, size.y as u32));
    }
}

/// Reallocate the render target when the requested size changes.
fn resize_target(mut stage: ResMut<PreviewStage>, mut images: ResMut<Assets<Image>>) {
    let want = stage.viewport;
    let Some(image) = images.get_mut(&stage.target) else {
        return;
    };
    if image.texture_descriptor.size.width == want.x
        && image.texture_descriptor.size.height == want.y
    {
        return;
    }
    image.resize(Extent3d {
        width: want.x,
        height: want.y,
        depth_or_array_layers: 1,
    });
    // Touch the stage so change detection sees the resize; the camera reads
    // `aspect()` and must not keep framing for the previous shape.
    stage.set_changed();
}

/// The stage: one camera, its lights, and whatever asset is on it.
#[derive(Resource)]
pub struct PreviewStage {
    /// Hang this in an `ImageNode` to show the preview.
    pub target: Handle<Image>,
    pub orbit: Orbit,
    /// Bounding radius of the current content; the camera frames from it.
    radius: f32,
    /// Spawned content, despawned wholesale when the selection changes.
    content: Vec<Entity>,
    /// Id of what is on the stage, so re-selecting the same asset does not
    /// rebuild it and reset the user's orbit mid-look.
    current: Option<String>,
    /// Spun slowly when the user is not dragging. A static render of a static
    /// mesh is hard to read as 3D at all.
    pub auto_spin: bool,
    /// Pixel size the render target should be, tracked so the image is only
    /// reallocated when it actually changes rather than every drag frame.
    viewport: UVec2,
    /// Materials built for each asset, kept across selections.
    ///
    /// Browsing A -> B -> A otherwise rebuilds identical materials every time
    /// the selection returns, which is pure churn on a screen whose whole job
    /// is flicking between assets. Textures are shared handles, so a cached
    /// entry costs almost nothing.
    material_cache: std::collections::HashMap<String, Vec<Handle<StandardMaterial>>>,
}

/// Smallest and largest render target the stage will allocate.
///
/// A dragged splitter can momentarily report a degenerate or enormous size;
/// neither should turn into a texture allocation.
const MIN_TARGET: u32 = 64;
const MAX_TARGET: u32 = 2048;

impl PreviewStage {
    /// What is currently staged.
    pub fn current(&self) -> Option<&str> {
        self.current.as_deref()
    }

    /// Aspect ratio of the current render target (width / height).
    pub fn aspect(&self) -> f32 {
        if self.viewport.y == 0 {
            1.0
        } else {
            self.viewport.x as f32 / self.viewport.y as f32
        }
    }

    /// Ask for a render target of this pixel size.
    ///
    /// Cheap to call every frame: it only records the request, and the resize
    /// system reallocates when the value actually differs.
    pub fn set_viewport(&mut self, size: UVec2) {
        let want = UVec2::new(
            size.x.clamp(MIN_TARGET, MAX_TARGET),
            size.y.clamp(MIN_TARGET, MAX_TARGET),
        );
        if want != self.viewport {
            self.viewport = want;
        }
    }

    /// Drop whatever is on the stage. Leaves the camera and lights up.
    pub fn clear(&mut self, commands: &mut Commands) {
        for entity in self.content.drain(..) {
            commands.entity(entity).despawn();
        }
        self.current = None;
    }

    /// Put an asset on the stage.
    ///
    /// `meshes` is every submesh, so a multi-part asset is shown whole.
    /// `bounds` is its local min/max; the camera frames from it, so the asset
    /// fills the viewport at any real-world scale.
    ///
    /// Re-staging the id already shown is a no-op — the browser calls this
    /// every frame from its selection, and rebuilding would both churn
    /// entities and fight the auto-spin.
    pub fn show(
        &mut self,
        commands: &mut Commands,
        id: &str,
        meshes: Vec<(Handle<Mesh>, Handle<StandardMaterial>)>,
        bounds: (Vec3, Vec3),
    ) {
        if self.current.as_deref() == Some(id) {
            return;
        }
        self.clear(commands);

        let (min, max) = bounds;
        let centre = (min + max) * 0.5;
        // Half the diagonal: no corner leaves frame at any orbit angle.
        self.radius = ((max - min).length() * 0.5).max(0.05);

        let layer = RenderLayers::layer(PREVIEW_LAYER);
        let pivot = commands
            .spawn((
                Transform::from_translation(-centre),
                Visibility::default(),
                layer.clone(),
            ))
            .id();
        for (mesh, material) in meshes {
            commands.entity(pivot).with_children(|p| {
                p.spawn((Mesh3d(mesh), MeshMaterial3d(material), layer.clone()));
            });
        }
        self.content.push(pivot);
        self.current = Some(id.to_string());
    }

    /// Stage an asset straight from scene-graph parts.
    ///
    /// This is the form a baked pack actually hands you — `MeshId` plus a
    /// [`MaterialDesc`] per submesh — and turning that into drawable Bevy
    /// handles is engine plumbing, not game logic. A game doing it itself has
    /// to know to resolve all four texture slots, and the copy that already
    /// existed bound base colour alone, which renders hard-surface art as a
    /// smooth shape with the detail painted on.
    ///
    /// Returns false if no submesh resolved, so a caller can leave the
    /// previous asset up rather than blanking the stage.
    pub fn show_scene_parts(
        &mut self,
        commands: &mut Commands,
        id: &str,
        parts: &[(artificer_scene::MeshId, artificer_scene::MaterialDesc)],
        bounds: ([f32; 3], [f32; 3]),
        maps: &artificer_render::AdapterMaps,
        materials: &mut Assets<StandardMaterial>,
    ) -> bool {
        if self.current.as_deref() == Some(id) {
            return true;
        }
        let cached = self.material_cache.get(id).cloned();
        let mut built: Vec<Handle<StandardMaterial>> = Vec::new();
        let mut drawn = Vec::new();
        for (index, (mesh_id, desc)) in parts.iter().enumerate() {
            let Some(mesh) = maps.meshes.get(mesh_id).cloned() else {
                continue;
            };
            let material = match cached.as_ref().and_then(|c| c.get(index)) {
                Some(handle) => handle.clone(),
                None => {
                    let handle = materials.add(artificer_render::material_from_desc(desc, maps));
                    built.push(handle.clone());
                    handle
                }
            };
            drawn.push((mesh, material));
        }
        if drawn.is_empty() {
            return false;
        }
        if cached.is_none() {
            self.material_cache.insert(id.to_string(), built);
        }
        self.show(
            commands,
            id,
            drawn,
            (Vec3::from_array(bounds.0), Vec3::from_array(bounds.1)),
        );
        true
    }
}

/// Build the stage: render target, camera, and a three-point rig.
fn setup_stage(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    // Square, and generous enough that the panel can scale it down rather than
    // up -- an upscaled preview looks worse than a small one.
    const SIZE: u32 = 768;
    let mut image = Image::new_fill(
        Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &[0, 0, 0, 0],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    image.texture_descriptor.usage =
        TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST | TextureUsages::RENDER_ATTACHMENT;
    let target = images.add(image);

    let layer = RenderLayers::layer(PREVIEW_LAYER);

    // Key, fill, rim -- same reasoning as the thumbnail rig: one light leaves
    // dark hull plating reading as a black silhouette, and most of this art is
    // deliberately dark.
    for (direction, colour, lux) in [
        (Vec3::new(0.72, 0.52, 1.0), Color::WHITE, 20_000.0),
        (
            Vec3::new(-0.8, 0.3, -0.6),
            Color::srgb(0.55, 0.7, 1.0),
            7_000.0,
        ),
        (
            Vec3::new(0.0, -0.6, -1.0),
            Color::srgb(1.0, 0.85, 0.7),
            4_000.0,
        ),
    ] {
        commands.spawn((
            DirectionalLight {
                color: colour,
                illuminance: lux,
                shadows_enabled: false,
                ..default()
            },
            Transform::from_translation(direction.normalize() * 10.0)
                .looking_at(Vec3::ZERO, Vec3::Y),
            layer.clone(),
        ));
    }

    commands.spawn((
        Camera3d::default(),
        Camera {
            target: RenderTarget::Image(target.clone().into()),
            clear_color: ClearColorConfig::Custom(Color::NONE),
            // Before the world camera, so the image is ready in the same frame
            // the panel samples it.
            order: -10,
            // Off unless something is staged: an asset browser is open for a
            // fraction of a session and this is a whole extra 3D pass.
            is_active: false,
            ..default()
        },
        Transform::default(),
        layer,
        PreviewCamera,
    ));

    commands.insert_resource(PreviewStage {
        target,
        orbit: Orbit::default(),
        radius: 1.0,
        content: Vec::new(),
        current: None,
        viewport: UVec2::splat(SIZE),
        auto_spin: true,
        material_cache: std::collections::HashMap::new(),
    });
}

/// Drive the camera from the orbit state, and idle-spin when nothing is
/// being dragged.
fn drive_camera(
    time: Res<Time>,
    mut stage: ResMut<PreviewStage>,
    mut cameras: Query<(&mut Transform, &mut Camera), With<PreviewCamera>>,
) {
    let staged = stage.current.is_some();
    if staged && stage.auto_spin {
        let step = time.delta_secs() * 0.35;
        stage.orbit.yaw += step;
    }
    let eye = stage.orbit.eye(stage.radius, stage.aspect());
    for (mut transform, mut camera) in &mut cameras {
        // Nothing staged means nothing to draw; leaving the pass on costs a
        // full render every frame for a transparent image.
        if camera.is_active != staged {
            camera.is_active = staged;
        }
        if staged {
            *transform = Transform::from_translation(eye).looking_at(Vec3::ZERO, Vec3::Y);
        }
    }
}

pub struct PreviewPlugin;

impl Plugin for PreviewPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_stage).add_systems(
            Update,
            // Size first, then frame from it: framing on last frame's aspect
            // makes the model visibly pop as a splitter is dragged.
            (follow_viewport_node, resize_target, drive_camera).chain(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pitch_is_clamped_short_of_the_poles() {
        let mut orbit = Orbit::default();
        orbit.turn(Vec2::new(0.0, 100.0));
        assert!(orbit.pitch < std::f32::consts::FRAC_PI_2);
        orbit.turn(Vec2::new(0.0, -200.0));
        assert!(orbit.pitch > -std::f32::consts::FRAC_PI_2);
    }

    #[test]
    fn zoom_is_bounded_in_both_directions() {
        let mut orbit = Orbit::default();
        for _ in 0..200 {
            orbit.zoom(1.0);
        }
        assert!(orbit.distance >= Orbit::MIN_DISTANCE);
        for _ in 0..400 {
            orbit.zoom(-1.0);
        }
        assert!(orbit.distance <= Orbit::MAX_DISTANCE);
    }

    #[test]
    fn framing_scales_with_the_model_so_any_size_asset_fills_the_view() {
        let orbit = Orbit::default();
        // A 200 m megastructure and a 40 cm relic must both fill the frame,
        // so the eye distance is strictly proportional to the radius.
        let small = orbit.eye(0.4, 1.0).length();
        let large = orbit.eye(200.0, 1.0).length();
        assert!(
            (large / small - 500.0).abs() < 0.5,
            "framing must be scale-free"
        );
    }

    #[test]
    fn a_narrow_viewport_pulls_the_camera_back_so_the_model_still_fits() {
        // This is the resize behaviour: drag the panel narrow and the model
        // has to shrink to stay inside, not spill out of the sides.
        let orbit = Orbit::default();
        let square = orbit.eye(1.0, 1.0).length();
        let narrow = orbit.eye(1.0, 0.4).length();
        let wide = orbit.eye(1.0, 2.5).length();
        assert!(narrow > square, "a tall thin view must pull back");
        // Beyond square the vertical angle is the tighter one, so widening
        // further must NOT keep pushing the camera away.
        assert!(
            (wide - square).abs() < 1e-3,
            "wide views stay vertically framed"
        );
    }

    #[test]
    fn fit_distance_puts_the_sphere_exactly_on_the_frame_edge() {
        let fov = std::f32::consts::FRAC_PI_4;
        let d = fit_distance(1.0, fov, 1.0);
        // At distance d the half-angle subtended by a unit sphere is
        // asin(1/d), which must equal the camera's half-fov.
        assert!(((1.0f32 / d).asin() - fov * 0.5).abs() < 1e-4);
    }

    #[test]
    fn yaw_wraps_without_changing_where_the_camera_is() {
        let mut a = Orbit::default();
        let before = a.eye(1.0, 1.0);
        a.yaw += std::f32::consts::TAU;
        let after = a.eye(1.0, 1.0);
        assert!((before - after).length() < 1e-3);
    }
}
