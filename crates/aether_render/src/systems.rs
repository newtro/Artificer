//! Adapter systems: input collection, game callbacks, scene mirroring.

use crate::convert::{to_bevy_mesh, to_bevy_transform, to_std_material, to_tonemapping};
use crate::keymap::KEY_PAIRS;
use crate::{EngineCtx, FrameInfo, GameRes, InputRes, SceneRes};
use aether_scene::{LightDesc, MeshId, NodeId, NodeKind, SceneCommand};
use bevy::core_pipeline::bloom::Bloom;
use bevy::input::mouse::{MouseMotion, MouseScrollUnit, MouseWheel};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use glam::Vec2 as GVec2;
use std::collections::HashMap;

/// Adapter bookkeeping: scene ids -> Bevy entities/assets.
#[derive(Resource, Default)]
pub(crate) struct AdapterMaps {
    pub nodes: HashMap<NodeId, Entity>,
    pub meshes: HashMap<MeshId, Handle<Mesh>>,
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

    use aether_input::MouseButton as AMouse;
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

fn with_ctx(world: &mut World, f: impl FnOnce(&mut dyn crate::GameClient, &mut EngineCtx)) {
    let dt = world.resource::<Time>().delta_secs();
    let elapsed = world.resource::<Time>().elapsed_secs();
    let window_size = world.resource::<FrameInfo>().window_size;
    world.resource_scope(|world, mut game: Mut<GameRes>| {
        world.resource_scope(|world, mut scene: Mut<SceneRes>| {
            world.resource_scope(|world, mut hud: Mut<crate::HudBoard>| {
                let input = world.resource::<InputRes>();
                let mut ctx = EngineCtx {
                    scene: &mut scene.0,
                    input: &input.0,
                    dt,
                    elapsed,
                    window_size,
                    hud: &mut hud,
                    exit_requested: false,
                };
                f(game.0.as_mut(), &mut ctx);
                if ctx.exit_requested {
                    world.send_event(bevy::app::AppExit::Success);
                }
            });
        });
    });
}

pub(crate) fn game_setup(world: &mut World) {
    with_ctx(world, |game, ctx| game.setup(ctx));
}

pub(crate) fn game_update(world: &mut World) {
    with_ctx(world, |game, ctx| game.update(ctx));
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
                world.resource_mut::<AdapterMaps>().meshes.insert(id, handle);
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
                        let mat_handle = world
                            .resource_mut::<Assets<StandardMaterial>>()
                            .add(to_std_material(&material));
                        let entity = world
                            .spawn((
                                Mesh3d(mesh_handle),
                                MeshMaterial3d(mat_handle.clone()),
                                to_bevy_transform(&transform),
                                Visibility::Inherited,
                            ))
                            .id();
                        world
                            .resource_mut::<AdapterMaps>()
                            .materials
                            .insert(id, mat_handle);
                        entity
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
                    let parent_entity = world.resource::<AdapterMaps>().nodes.get(&parent_id).copied();
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
                    let mut materials = world.resource_mut::<Assets<StandardMaterial>>();
                    if let Some(mat) = materials.get_mut(&handle) {
                        *mat = to_std_material(&material);
                    }
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

pub(crate) fn sync_active_camera(
    maps: Res<AdapterMaps>,
    mut cameras: Query<(Entity, &mut Camera)>,
) {
    let active_entity = maps
        .active_camera
        .and_then(|id| maps.camera_nodes.get(&id))
        .copied();
    for (entity, mut camera) in cameras.iter_mut() {
        let want = Some(entity) == active_entity;
        if camera.is_active != want {
            camera.is_active = want;
        }
    }
}
