//! Adapter systems: input collection, game callbacks, scene mirroring.

use crate::convert::{
    decode_texture, to_bevy_mesh, to_bevy_transform, to_std_material, to_tonemapping,
};
use crate::keymap::KEY_PAIRS;
use crate::{EngineCtx, FrameInfo, GameRes, InputRes, SceneRes};
use artificer_scene::{LightDesc, MaterialDesc, MeshId, NodeId, NodeKind, SceneCommand, TextureId};
use bevy::core_pipeline::bloom::Bloom;
use bevy::input::mouse::{MouseMotion, MouseScrollUnit, MouseWheel};
use bevy::pbr::NotShadowCaster;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use glam::Vec2 as GVec2;
use std::collections::HashMap;

/// Adapter bookkeeping: scene ids -> Bevy entities/assets.
#[derive(Resource, Default)]
pub struct AdapterMaps {
    pub nodes: HashMap<NodeId, Entity>,
    pub meshes: HashMap<MeshId, Handle<Mesh>>,
    pub textures: HashMap<TextureId, Handle<Image>>,
    pub materials: HashMap<NodeId, Handle<StandardMaterial>>,
    pub camera_nodes: HashMap<NodeId, Entity>,
    pub active_camera: Option<NodeId>,
}

pub(crate) fn collect_input(
    mut input: ResMut<InputRes>,
    mut frame: ResMut<FrameInfo>,
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    mut motion: EventReader<MouseMotion>,
    mut wheel: EventReader<MouseWheel>,
    windows: Query<&Window, With<PrimaryWindow>>,
) {
    let state = &mut input.0;
    state.begin_frame();

    for (key, code) in KEY_PAIRS {
        if keys.just_pressed(*code) {
            state.press(*key);
        }
        if keys.just_released(*code) {
            state.release(*key);
        }
    }

    use artificer_input::MouseButton as AMouse;
    let mouse_pairs = [
        (AMouse::Left, MouseButton::Left),
        (AMouse::Right, MouseButton::Right),
        (AMouse::Middle, MouseButton::Middle),
    ];
    for (am, bm) in mouse_pairs {
        if buttons.just_pressed(bm) {
            state.press_mouse(am);
        }
        if buttons.just_released(bm) {
            state.release_mouse(am);
        }
    }

    for ev in motion.read() {
        state.add_mouse_delta(GVec2::new(ev.delta.x, ev.delta.y));
    }
    for ev in wheel.read() {
        let amount = match ev.unit {
            MouseScrollUnit::Line => ev.y,
            MouseScrollUnit::Pixel => ev.y / 100.0,
        };
        state.add_wheel(amount);
    }

    if let Ok(window) = windows.single() {
        frame.window_size = GVec2::new(window.width(), window.height());
        if let Some(pos) = window.cursor_position() {
            state.mouse_position = GVec2::new(pos.x, pos.y);
        }
    }
}

/// Build a world-space ray through the cursor from the camera that is
/// actually drawing the scene.
///
/// Deliberately the SCENE camera from `AdapterMaps`, not "any active camera":
/// a game with a pinned HUD overlay has more than one, and picking through
/// the overlay camera would return rays into the instrument panel. That exact
/// confusion already caused world labels to drift once.
fn cursor_ray_from_scene_camera(
    world: &mut World,
    cursor: glam::Vec2,
) -> Option<(glam::Vec3, glam::Vec3)> {
    let owned: Vec<Entity> = world
        .resource::<AdapterMaps>()
        .camera_nodes
        .values()
        .copied()
        .collect();
    let mut cameras = world.query::<(Entity, &Camera, &GlobalTransform)>();
    for (entity, camera, transform) in cameras.iter(world) {
        if !owned.contains(&entity) || !camera.is_active {
            continue;
        }
        let ray = camera
            .viewport_to_world(transform, Vec2::new(cursor.x, cursor.y))
            .ok()?;
        return Some((
            glam::Vec3::new(ray.origin.x, ray.origin.y, ray.origin.z),
            glam::Vec3::new(ray.direction.x, ray.direction.y, ray.direction.z),
        ));
    }
    None
}

fn with_ctx(world: &mut World, f: impl FnOnce(&mut dyn crate::GameClient, &mut EngineCtx)) {
    let dt = world.resource::<Time>().delta_secs();
    let elapsed = world.resource::<Time>().elapsed_secs();
    let window_size = world.resource::<FrameInfo>().window_size;
    // GameRes is non-send (game code owns sockets/JS handles); temporarily
    // detach it from the world while it borrows the other resources.
    let mut game = world
        .remove_non_send_resource::<GameRes>()
        .expect("GameRes present");
    world.resource_scope(|world, mut scene: Mut<SceneRes>| {
        world.resource_scope(|world, mut hud: Mut<crate::HudBoard>| {
            world.resource_scope(|world, mut labels: Mut<crate::WorldLabels>| {
                labels.0.clear();
                // Read the cursor out before the ray query, so the &World
                // borrow for InputRes does not outlive the &mut World it needs.
                let cursor = world.resource::<InputRes>().0.mouse_position;
                let cursor_ray = cursor_ray_from_scene_camera(world, cursor);
                let input = world.resource::<InputRes>();
                let mut ctx = EngineCtx {
                    scene: &mut scene.0,
                    input: &input.0,
                    dt,
                    elapsed,
                    window_size,
                    hud: &mut hud,
                    labels: &mut labels,
                    cursor_ray,
                    exit_requested: false,
                    cursor_grab_request: None,
                };
                f(game.0.as_mut(), &mut ctx);
                let exit_requested = ctx.exit_requested;
                let cursor_grab_request = ctx.cursor_grab_request;
                if exit_requested {
                    world.send_event(bevy::app::AppExit::Success);
                }
                if let Some(grab) = cursor_grab_request {
                    // Read first: taking it mutably would flag the resource
                    // as changed every frame even when nothing moved.
                    if world.resource::<crate::CursorGrab>().0 != grab {
                        world.resource_mut::<crate::CursorGrab>().0 = grab;
                    }
                }
            });
        });
    });
    world.insert_non_send_resource(game);
}

pub(crate) fn game_setup(world: &mut World) {
    with_ctx(world, |game, ctx| game.setup(ctx));
}

pub(crate) fn game_update(world: &mut World) {
    with_ctx(world, |game, ctx| game.update(ctx));
}

/// Look up the image handle a material's texture id refers to.
///
/// An id with no registered texture warns rather than silently rendering
/// untextured: "the atlas did not load" and "this material has no atlas" look
/// identical on screen and take very different fixes.
/// Every image handle a material needs, resolved in one pass.
///
/// Resolved together and applied together, because these are the maps that
/// make a surface look like itself: base colour alone renders a hard-surface
/// asset as a smooth shape with its panel lines painted on. Binding them at
/// one site means adding a map cannot leave a second call path still binding
/// only the first.
#[derive(Default)]
struct MaterialMaps {
    base_color: Option<Handle<Image>>,
    normal: Option<Handle<Image>>,
    metallic_roughness: Option<Handle<Image>>,
    occlusion: Option<Handle<Image>>,
    emissive: Option<Handle<Image>>,
}

fn resolve_texture(world: &World, material: &MaterialDesc) -> MaterialMaps {
    let textures = &world.resource::<AdapterMaps>().textures;
    let one = |slot: Option<artificer_scene::TextureId>| match slot {
        None => None,
        Some(id) => match textures.get(&id) {
            Some(handle) => Some(handle.clone()),
            None => {
                log::warn!("material references texture {id:?}, which was never registered");
                None
            }
        },
    };
    MaterialMaps {
        base_color: one(material.base_color_texture),
        normal: one(material.normal_texture),
        metallic_roughness: one(material.metallic_roughness_texture),
        occlusion: one(material.occlusion_texture),
        emissive: one(material.emissive_texture),
    }
}

fn apply_maps(target: &mut StandardMaterial, maps: MaterialMaps) {
    target.base_color_texture = maps.base_color;
    target.normal_map_texture = maps.normal;
    target.metallic_roughness_texture = maps.metallic_roughness;
    target.occlusion_texture = maps.occlusion;
    target.emissive_texture = maps.emissive;
}

pub(crate) fn apply_scene_commands(world: &mut World) {
    let commands: Vec<SceneCommand> = {
        let mut scene = world.resource_mut::<SceneRes>();
        scene.0.drain_commands()
    };
    if commands.is_empty() {
        return;
    }

    for command in commands {
        match command {
            SceneCommand::AddMesh { id, data } => {
                let handle = world
                    .resource_mut::<Assets<Mesh>>()
                    .add(to_bevy_mesh(&data));
                world
                    .resource_mut::<AdapterMaps>()
                    .meshes
                    .insert(id, handle);
            }
            SceneCommand::AddTexture {
                id,
                png,
                sampling,
                color_space,
            } => {
                match decode_texture(&png, sampling, color_space) {
                    Ok(image) => {
                        let handle = world.resource_mut::<Assets<Image>>().add(image);
                        world
                            .resource_mut::<AdapterMaps>()
                            .textures
                            .insert(id, handle);
                    }
                    // A texture that will not decode must be loud. Silently
                    // leaving it unregistered gives an untextured -- not
                    // obviously broken -- surface, which is far harder to
                    // trace back than a line in the log.
                    Err(e) => log::error!("texture {id:?} failed to decode: {e}"),
                }
            }
            SceneCommand::Spawn {
                id,
                parent,
                transform,
                kind,
            } => {
                let entity = match kind {
                    NodeKind::Mesh { mesh, material } => {
                        let mesh_handle = match world.resource::<AdapterMaps>().meshes.get(&mesh) {
                            Some(h) => h.clone(),
                            None => {
                                log::warn!("spawn references unknown mesh {mesh:?}");
                                continue;
                            }
                        };
                        let maps = resolve_texture(world, &material);
                        let mut std_material = to_std_material(&material);
                        apply_maps(&mut std_material, maps);
                        let mat_handle = world
                            .resource_mut::<Assets<StandardMaterial>>()
                            .add(std_material);
                        let mut spawned = world.spawn((
                            Mesh3d(mesh_handle),
                            MeshMaterial3d(mat_handle.clone()),
                            to_bevy_transform(&transform),
                            Visibility::Inherited,
                        ));
                        if !material.casts_shadows {
                            spawned.insert(NotShadowCaster);
                        }
                        let entity = spawned.id();
                        world
                            .resource_mut::<AdapterMaps>()
                            .materials
                            .insert(id, mat_handle);
                        entity
                    }
                    NodeKind::Atmosphere { mesh, atmosphere } => {
                        let mesh_handle = match world.resource::<AdapterMaps>().meshes.get(&mesh) {
                            Some(h) => h.clone(),
                            None => {
                                log::warn!("atmosphere references unknown mesh {mesh:?}");
                                continue;
                            }
                        };
                        let material =
                            crate::atmosphere::AtmosphereMaterial::from_desc(&atmosphere);
                        let mat_handle = world
                            .resource_mut::<Assets<crate::atmosphere::AtmosphereMaterial>>()
                            .add(material);
                        // A shell of added light: never a shadow caster, and
                        // there is nothing PBR to track in `materials`.
                        world
                            .spawn((
                                Mesh3d(mesh_handle),
                                MeshMaterial3d(mat_handle),
                                to_bevy_transform(&transform),
                                Visibility::Inherited,
                                NotShadowCaster,
                            ))
                            .id()
                    }
                    NodeKind::Light(light) => match light {
                        LightDesc::Directional {
                            color,
                            illuminance,
                            shadows,
                        } => world
                            .spawn((
                                DirectionalLight {
                                    color: Color::srgb(color[0], color[1], color[2]),
                                    illuminance,
                                    shadows_enabled: shadows,
                                    ..Default::default()
                                },
                                to_bevy_transform(&transform),
                            ))
                            .id(),
                        LightDesc::Point {
                            color,
                            intensity,
                            range,
                            shadows,
                        } => world
                            .spawn((
                                PointLight {
                                    color: Color::srgb(color[0], color[1], color[2]),
                                    intensity,
                                    range,
                                    shadows_enabled: shadows,
                                    ..Default::default()
                                },
                                to_bevy_transform(&transform),
                            ))
                            .id(),
                    },
                    NodeKind::Camera(cam) => {
                        let mut entity_commands = world.spawn((
                            Camera3d::default(),
                            Camera {
                                hdr: cam.hdr,
                                is_active: false,
                                ..Default::default()
                            },
                            Projection::Perspective(PerspectiveProjection {
                                fov: cam.fov_y_degrees.to_radians(),
                                near: cam.near,
                                far: cam.far,
                                ..Default::default()
                            }),
                            to_tonemapping(cam.tonemapping),
                            to_bevy_transform(&transform),
                        ));
                        if let Some(bloom) = cam.bloom {
                            entity_commands.insert(Bloom {
                                intensity: bloom.intensity,
                                ..Bloom::NATURAL
                            });
                        }
                        let entity = entity_commands.id();
                        let mut maps = world.resource_mut::<AdapterMaps>();
                        maps.camera_nodes.insert(id, entity);
                        // First camera becomes active unless one was chosen.
                        if maps.active_camera.is_none() {
                            maps.active_camera = Some(id);
                        }
                        entity
                    }
                    NodeKind::Group => world
                        .spawn((to_bevy_transform(&transform), Visibility::Inherited))
                        .id(),
                };

                if let Some(parent_id) = parent {
                    let parent_entity = world
                        .resource::<AdapterMaps>()
                        .nodes
                        .get(&parent_id)
                        .copied();
                    match parent_entity {
                        Some(pe) => {
                            world.entity_mut(pe).add_child(entity);
                        }
                        None => log::warn!("spawn parent {parent_id:?} unknown"),
                    }
                }
                world.resource_mut::<AdapterMaps>().nodes.insert(id, entity);
            }
            SceneCommand::SetTransform { id, transform } => {
                if let Some(entity) = world.resource::<AdapterMaps>().nodes.get(&id).copied() {
                    if let Some(mut t) = world.get_mut::<Transform>(entity) {
                        *t = to_bevy_transform(&transform);
                    }
                }
            }
            SceneCommand::SetVisible { id, visible } => {
                if let Some(entity) = world.resource::<AdapterMaps>().nodes.get(&id).copied() {
                    if let Some(mut v) = world.get_mut::<Visibility>(entity) {
                        *v = if visible {
                            Visibility::Inherited
                        } else {
                            Visibility::Hidden
                        };
                    }
                }
            }
            SceneCommand::SetMaterial { id, material } => {
                let handle = world.resource::<AdapterMaps>().materials.get(&id).cloned();
                if let Some(handle) = handle {
                    // Resolve BEFORE borrowing the asset store, and apply the
                    // same binding as the spawn path -- otherwise changing a
                    // material at runtime silently drops its texture.
                    let maps = resolve_texture(world, &material);
                    let mut materials = world.resource_mut::<Assets<StandardMaterial>>();
                    if let Some(mat) = materials.get_mut(&handle) {
                        *mat = to_std_material(&material);
                        apply_maps(mat, maps);
                        // Shadow casting is a component, not a material field,
                        // so replacing the material alone left the old marker
                        // in place and the change had no effect.
                        if let Some(entity) =
                            world.resource::<AdapterMaps>().nodes.get(&id).copied()
                        {
                            let mut entity = world.entity_mut(entity);
                            if material.casts_shadows {
                                entity.remove::<NotShadowCaster>();
                            } else {
                                entity.insert(NotShadowCaster);
                            }
                        }
                    }
                }
            }
            SceneCommand::RemoveMesh { id } => {
                // Dropping the map's handle is the whole job: entities
                // still using the mesh hold their own strong handles, so
                // Bevy frees the GPU asset when the last of those despawns.
                if world
                    .resource_mut::<AdapterMaps>()
                    .meshes
                    .remove(&id)
                    .is_none()
                {
                    log::warn!("remove of unregistered mesh {id:?} (double release?)");
                }
            }
            SceneCommand::RemoveTexture { id } => {
                if world
                    .resource_mut::<AdapterMaps>()
                    .textures
                    .remove(&id)
                    .is_none()
                {
                    log::warn!("remove of unregistered texture {id:?} (double release?)");
                }
            }
            SceneCommand::Despawn { id } => {
                let removed = {
                    let mut maps = world.resource_mut::<AdapterMaps>();
                    maps.materials.remove(&id);
                    maps.camera_nodes.remove(&id);
                    if maps.active_camera == Some(id) {
                        maps.active_camera = None;
                    }
                    maps.nodes.remove(&id)
                };
                if let Some(entity) = removed {
                    if let Ok(entity_mut) = world.get_entity_mut(entity) {
                        entity_mut.despawn();
                    }
                }
            }
            SceneCommand::SetActiveCamera { id } => {
                world.resource_mut::<AdapterMaps>().active_camera = Some(id);
            }
            SceneCommand::SetEnvironment { env } => {
                world.resource_mut::<ClearColor>().0 = Color::srgba(
                    env.clear_color[0],
                    env.clear_color[1],
                    env.clear_color[2],
                    env.clear_color[3],
                );
                world.insert_resource(AmbientLight {
                    color: Color::srgb(
                        env.ambient_color[0],
                        env.ambient_color[1],
                        env.ambient_color[2],
                    ),
                    brightness: env.ambient_brightness,
                    ..Default::default()
                });
            }
        }
    }
}

pub(crate) fn apply_cursor_grab(
    grab: Res<crate::CursorGrab>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
) {
    if !grab.is_changed() {
        return;
    }
    let Ok(mut window) = windows.single_mut() else {
        return;
    };
    use bevy::window::CursorGrabMode;
    // Platforms disagree about which grab they implement: winit supports
    // `Locked` on macOS/web and `Confined` on Windows/X11. Asking for the
    // unsupported one fails silently and leaves the pointer free to wander
    // off the window mid-turn, which reads as stuttering. Steering uses raw
    // motion deltas, so `Confined` is equally good where it is the supported
    // mode.
    let want_mode = match (
        grab.0,
        cfg!(any(target_os = "macos", target_family = "wasm")),
    ) {
        (false, _) => CursorGrabMode::None,
        (true, true) => CursorGrabMode::Locked,
        (true, false) => CursorGrabMode::Confined,
    };
    // Compare before assigning: `Mut` marks the window changed on any
    // mutable deref, and a window marked dirty every frame makes the winit
    // adapter re-diff it every frame.
    if window.cursor_options.grab_mode != want_mode {
        window.cursor_options.grab_mode = want_mode;
    }
    if window.cursor_options.visible == grab.0 {
        window.cursor_options.visible = !grab.0;
    }
}

pub(crate) fn sync_active_camera(
    mut commands: Commands,
    maps: Res<AdapterMaps>,
    mut cameras: Query<(Entity, &mut Camera, Has<IsDefaultUiCamera>)>,
) {
    let active_entity = maps
        .active_camera
        .and_then(|id| maps.camera_nodes.get(&id))
        .copied();
    // Only the cameras this adapter created. A game may legitimately own
    // others — a pinned HUD pass, a render-target camera behind a world-space
    // UI panel — and switching those off every frame because they are not the
    // scene's active camera makes them impossible to use at all.
    let owned: std::collections::HashSet<Entity> = maps.camera_nodes.values().copied().collect();
    for (entity, mut camera, is_ui_default) in cameras.iter_mut() {
        if !owned.contains(&entity) {
            continue;
        }
        let want = Some(entity) == active_entity;
        if camera.is_active != want {
            camera.is_active = want;
        }
        // UI always follows the active camera, otherwise Bevy UI cannot
        // decide which of several cameras to render onto.
        if want && !is_ui_default {
            commands.entity(entity).insert(IsDefaultUiCamera);
        } else if !want && is_ui_default {
            commands.entity(entity).remove::<IsDefaultUiCamera>();
        }
    }
}
