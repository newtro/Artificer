//! Small 3D renders of meshes, for use as icons in UI.
//!
//! An inventory that lists its contents by name is a spreadsheet. This turns
//! a mesh into a `Handle<Image>` you can drop into an `ImageNode`, so a parts
//! list can show the part.
//!
//! Each thumbnail gets its own camera on a shared render layer, framed on the
//! asset's bounds and rendered to its own texture. The capture then tears
//! itself down: a static mesh under static lighting looks the same on frame
//! two as on frame two thousand, and leaving dozens of cameras and lights
//! resident would cost a draw pass each, every frame, forever. The image
//! outlives the scene that produced it.

use bevy::asset::RenderAssetUsages;
use bevy::image::Image;
use bevy::prelude::*;
use bevy::render::camera::RenderTarget;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};
use bevy::render::view::RenderLayers;

/// First layer thumbnails render on.
///
/// Chosen to leave room for `THUMBNAIL_LAYER_COUNT` layers BELOW the reserved
/// ones: [`crate::HUD_LAYER`] is 30 and the panel UI layer is 31, so a range
/// ending at 31 would have put two captures onto live UI passes.
///
/// Captures get a layer EACH, rotating through a pool. Sharing one layer put
/// every pending mesh into every icon at once, because a camera sees its whole
/// layer and several captures overlap in time.
pub const THUMBNAIL_LAYER: usize = 16;

/// How many capture layers exist.
///
/// LEASED, not rotated. Rotating meant a sixth concurrent capture silently
/// reused a live layer and both meshes landed in both icons. Asking for one
/// while all are out returns `None` and the caller tries again next frame.
const THUMBNAIL_LAYER_COUNT: usize = 8;

/// Which capture layers are currently in use.
#[derive(Resource, Default)]
pub struct ThumbnailLayers {
    leased: [bool; THUMBNAIL_LAYER_COUNT],
}

impl ThumbnailLayers {
    fn take(&mut self) -> Option<usize> {
        let slot = self.leased.iter().position(|used| !used)?;
        self.leased[slot] = true;
        Some(slot)
    }

    fn give_back(&mut self, slot: usize) {
        if let Some(used) = self.leased.get_mut(slot) {
            *used = false;
        }
    }
}

/// Frames a capture stays live before it is torn down.
///
/// More than one because the first frame can land before the mesh's asset is
/// ready; a handful is cheap and removes the class of bug where an icon is
/// permanently blank because its single render happened too early.
const WARMUP_FRAMES: u32 = 8;

/// A capture in progress.
#[derive(Component)]
pub struct ThumbnailCamera {
    frames: u32,
    /// Everything spawned to take this one picture, so it can all be removed
    /// once the picture is taken. The caller only ever sees the image.
    staging: Vec<Entity>,
    /// The layer lease to return when this capture finishes.
    slot: usize,
}

/// Retire finished captures.
pub(crate) fn tick_thumbnail_cameras(
    mut commands: Commands,
    mut leases: ResMut<ThumbnailLayers>,
    mut cameras: Query<(Entity, &mut ThumbnailCamera)>,
) {
    for (entity, mut thumbnail) in &mut cameras {
        if thumbnail.frames < WARMUP_FRAMES {
            thumbnail.frames += 1;
            continue;
        }
        for staged in thumbnail.staging.drain(..) {
            commands.entity(staged).despawn();
        }
        leases.give_back(thumbnail.slot);
        commands.entity(entity).despawn();
    }
}

/// Render one asset to a texture and return it.
///
/// `meshes` is every submesh of the asset, so a model built from several
/// pieces is drawn whole rather than as whichever piece happened to be first.
///
/// `bounds` is the asset's own min/max in its local space; the camera is
/// framed from it, so a 50 m freighter part and a 60 cm greeble both fill
/// their icon instead of one being a dot and the other clipping.
///
/// The asset is viewed three-quarter-on rather than square-on, because a
/// front-on render of most hardware reads as an ambiguous blob.
pub fn render_thumbnail(
    commands: &mut Commands,
    images: &mut Assets<Image>,
    leases: &mut ThumbnailLayers,
    meshes: Vec<(Handle<Mesh>, Handle<StandardMaterial>)>,
    bounds: (Vec3, Vec3),
    size: u32,
    key_light: Color,
) -> Option<Handle<Image>> {
    let mut image = Image::new_fill(
        Extent3d {
            width: size,
            height: size,
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

    let (min, max) = bounds;
    let centre = (min + max) * 0.5;
    // Half the diagonal, so no corner escapes the frame from any angle.
    let radius = ((max - min).length() * 0.5).max(0.05);

    let slot = leases.take()?;
    let layer = RenderLayers::layer(THUMBNAIL_LAYER + slot);
    let mut staging = Vec::new();

    let pivot = commands
        .spawn((
            Transform::from_translation(-centre),
            Visibility::default(),
            layer.clone(),
        ))
        .id();
    for (mesh, material) in meshes {
        commands.entity(pivot).with_children(|p| {
            p.spawn((
                Mesh3d(mesh),
                MeshMaterial3d(material),
                Transform::IDENTITY,
                layer.clone(),
            ));
        });
    }
    staging.push(pivot);

    // Three-quarter view, from slightly above.
    let eye = Vec3::new(0.72, 0.52, 1.0).normalize() * radius * 2.6;

    // Key, fill and rim. A single light left dark hull plating reading as a
    // black rectangle -- an icon has to be legible at 46 px against whatever
    // the panel behind it happens to be.
    for (direction, colour, lux) in [
        (eye, key_light, 26_000.0),
        (Vec3::new(-eye.x, eye.y * 0.4, -eye.z), key_light, 9_000.0),
        (Vec3::new(0.0, -eye.y, -eye.z), Color::WHITE, 5_000.0),
    ] {
        let light = commands
            .spawn((
                DirectionalLight {
                    color: colour,
                    illuminance: lux,
                    shadows_enabled: false,
                    ..default()
                },
                Transform::from_translation(direction).looking_at(Vec3::ZERO, Vec3::Y),
                layer.clone(),
            ))
            .id();
        staging.push(light);
    }

    commands.spawn((
        Camera3d::default(),
        Camera {
            target: RenderTarget::Image(target.clone().into()),
            clear_color: ClearColorConfig::Custom(Color::NONE),
            // Ahead of the scene camera, so the capture is complete before the
            // frame that uses it and never draws over the world itself.
            order: -20,
            ..default()
        },
        Transform::from_translation(eye).looking_at(Vec3::ZERO, Vec3::Y),
        layer,
        ThumbnailCamera {
            frames: 0,
            staging,
            slot,
        },
    ));

    Some(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_finished_capture_tears_itself_down() {
        let mut app = App::new();
        app.init_resource::<ThumbnailLayers>();
        app.add_systems(Update, tick_thumbnail_cameras);

        let staged = app.world_mut().spawn_empty().id();
        let camera = app
            .world_mut()
            .spawn(ThumbnailCamera {
                frames: 0,
                staging: vec![staged],
                slot: 0,
            })
            .id();

        for _ in 0..WARMUP_FRAMES {
            app.update();
            assert!(
                app.world().get_entity(camera).is_ok(),
                "must keep capturing through the warm-up, or icons can be blank"
            );
        }
        app.update();

        assert!(
            app.world().get_entity(camera).is_err(),
            "a finished capture must not linger and cost a draw pass forever"
        );
        assert!(
            app.world().get_entity(staged).is_err(),
            "its mesh and light must go with it"
        );
    }

    /// The capture range must not touch a layer the engine already uses.
    #[test]
    fn thumbnail_layers_avoid_reserved_ones() {
        let last = THUMBNAIL_LAYER + THUMBNAIL_LAYER_COUNT - 1;
        assert!(
            last < crate::HUD_LAYER,
            "captures reach layer {last}, which collides with the HUD ({}) or \
             the panel UI above it",
            crate::HUD_LAYER
        );
    }

    /// Layers are leased, so a capture can never be handed a layer another
    /// capture is still drawing into.
    #[test]
    fn layers_are_leased_and_returned() {
        let mut leases = ThumbnailLayers::default();
        let taken: Vec<_> = (0..THUMBNAIL_LAYER_COUNT)
            .map(|_| leases.take().expect("a free layer"))
            .collect();
        assert_eq!(
            taken.len(),
            THUMBNAIL_LAYER_COUNT,
            "every layer should be available to start with"
        );
        let mut seen = taken.clone();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), taken.len(), "no layer may be leased twice");

        assert!(
            leases.take().is_none(),
            "asking with none free must refuse, not reuse a live layer"
        );
        leases.give_back(taken[2]);
        assert_eq!(
            leases.take(),
            Some(taken[2]),
            "a returned layer is reusable"
        );
    }
}
