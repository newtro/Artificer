//! World-space UI panel demo: the same three panels under each skin.
//!
//! Everything on screen is geometry. The panels are meshes standing in a
//! lit scene with a rotating solid between them, so you can see them catch
//! bloom, occlude and be occluded, and hold their shape as the camera moves.
//!
//! SPACE cycles the skin. LEFT/RIGHT orbits. ESC exits.

mod hud;

use artificer_ui::{
    spawn_panel, ActiveSkin, ArtificerUiPlugin, BlankTexture, PanelDesc, PanelMaterial, Skin,
    SkinId, SkinRegistry, TexturedSkin,
};
use bevy::core_pipeline::bloom::Bloom;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::prelude::*;
use bevy::render::view::screenshot::{save_to_disk, Screenshot};

/// `--shot <path>`: render a few frames, save a PNG, exit.
///
/// Deterministic captures beat driving the window manager, and the same hook
/// gives visual regression something to diff against later.
#[derive(Resource)]
struct ShotRequest {
    path: String,
    frames: u32,
}

fn arg(name: &str) -> Option<String> {
    args_all(name).into_iter().next()
}

/// Every value given for a repeated flag, so `--skin-dir a --skin-dir b`
/// puts both art skins in the same cycle for side-by-side comparison.
fn args_all(name: &str) -> Vec<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .enumerate()
        .filter(|(_, a)| *a == name)
        .filter_map(|(i, _)| args.get(i + 1).cloned())
        .collect()
}

/// Per-directory styling. The art decides: white frames want the accent,
/// coloured frames want to be left alone.
struct SkinStyle {
    accent: Color,
    backdrop: Color,
    frame_tint: Color,
    source_border: Vec2,
    panel_border: Vec2,
    content_inset: Vec2,
    emissive: f32,
}

fn style_for(dir: &str) -> SkinStyle {
    if dir.to_lowercase().contains("scifi") {
        // Teal-and-gold bulkhead art, 1024px, border 126x161 with corner
        // brackets reaching further in than the edge struts.
        SkinStyle {
            accent: Color::srgb(0.45, 0.80, 0.95),
            backdrop: Color::srgb(0.035, 0.055, 0.075),
            frame_tint: Color::srgb(1.0, 1.0, 1.0),
            source_border: Vec2::splat(0.20),
            panel_border: Vec2::new(0.10, 0.15),
            content_inset: Vec2::new(0.10, 0.15),
            emissive: 1.9,
        }
    } else {
        // Near-white chamfered bevel, 512px, bevel x=39..92, chamfer to 84.
        SkinStyle {
            accent: Color::srgb(0.62, 0.86, 1.0),
            backdrop: Color::srgb(0.045, 0.06, 0.085),
            frame_tint: Color::srgb(0.62, 0.86, 1.0),
            source_border: Vec2::splat(0.20),
            panel_border: Vec2::new(0.085, 0.13),
            content_inset: Vec2::new(0.075, 0.11),
            emissive: 1.9,
        }
    }
}

fn skin_from_args() -> Skin {
    match arg("--skin").unwrap_or_default().to_lowercase().as_str() {
        "industrial" => Skin::Industrial,
        "minimal" => Skin::Minimal,
        _ => Skin::Holographic,
    }
}

/// Load a textured skin from a directory of PNGs.
///
/// `--skin-dir <path>` exists so licensed art can be previewed without a byte
/// of it entering this repository: the game keeps its own art and points the
/// demo at it.
fn load_skin_dir(
    dir: &str,
    images: &mut Assets<Image>,
    registry: &mut SkinRegistry,
) -> Option<SkinId> {
    let load = |images: &mut Assets<Image>, name: &str| -> Option<Handle<Image>> {
        let path = std::path::Path::new(dir).join(name);
        let bytes = std::fs::read(&path).ok()?;
        let image = Image::from_buffer(
            &bytes,
            bevy::image::ImageType::Extension("png"),
            bevy::image::CompressedImageFormats::NONE,
            true,
            bevy::image::ImageSampler::linear(),
            bevy::asset::RenderAssetUsages::default(),
        )
        .ok()?;
        Some(images.add(image))
    };
    let frame = load(images, "frame.png")?;
    let background = load(images, "frame_background.png");
    let frame_selected = load(images, "frame_selected.png");
    let name = std::path::Path::new(dir)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "Textured".to_string());
    let st = style_for(dir);
    Some(registry.register(TexturedSkin {
        name,
        frame,
        frame_selected,
        background,
        frame_tint: st.frame_tint,
        source_border: st.source_border,
        panel_border: st.panel_border,
        params: artificer_ui::SkinParams {
            accent: st.accent,
            text: Color::srgb(0.95, 0.97, 1.0),
            dim_text: Color::srgb(0.55, 0.63, 0.72),
            backdrop: st.backdrop,
            emissive: st.emissive,
            backdrop_opacity: 1.0,
            scanline_strength: 0.0,
            edge_glow: 0.45,
            flicker: 0.0,
            bezel: 0.0,
            curvature: 0.0,
            corner_radius: 0.0,
            content_inset: st.content_inset,
        },
    }))
}

#[derive(Resource)]
struct Orbit {
    angle: f32,
}

/// Marks the label that reports the active skin, so it can be retitled.
#[derive(Component)]
struct SkinLabel;

fn main() {
    let mut app = App::new();
    if let Some(path) = arg("--shot") {
        app.insert_resource(ShotRequest { path, frames: 0 });
    }
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "Artificer — world-space UI panels".to_string(),
            ..default()
        }),
        ..default()
    }))
    .add_plugins(ArtificerUiPlugin)
    .insert_resource(Orbit { angle: 0.0 })
    .insert_resource(ActiveSkin(SkinId::Builtin(skin_from_args())))
    .insert_resource(ClearColor(Color::srgb(0.008, 0.012, 0.02)))
    .add_systems(Startup, setup)
    .add_systems(Update, (cycle_skin, orbit_camera, spin_prop, take_shot))
    .run();
}

/// Save after enough frames that the render target and bloom have settled.
fn take_shot(
    mut commands: Commands,
    shot: Option<ResMut<ShotRequest>>,
    mut exit: EventWriter<AppExit>,
) {
    let Some(mut shot) = shot else { return };
    shot.frames += 1;
    if shot.frames == 40 {
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(shot.path.clone()));
    }
    if shot.frames > 60 {
        exit.write(AppExit::Success);
    }
}

#[derive(Component)]
struct Prop;

/// Which mock scene to build.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Layout {
    /// Three panels in an arc: judging the panel material itself.
    Gallery,
    /// A cockpit HUD wrapped around the viewer: judging it as a HUD.
    Hud,
}

fn layout_from_args() -> Layout {
    match arg("--layout").unwrap_or_default().to_lowercase().as_str() {
        "hud" => Layout::Hud,
        _ => Layout::Gallery,
    }
}

#[allow(clippy::too_many_arguments)]
fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut panel_materials: ResMut<Assets<PanelMaterial>>,
    mut std_materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut registry: ResMut<SkinRegistry>,
    mut active: ResMut<ActiveSkin>,
    blank: Res<BlankTexture>,
) {
    let layout = layout_from_args();
    // A HUD is seen from inside the ship: the eye sits at the origin looking
    // down -Z, which is where the panels are arranged around. Viewing it from
    // the gallery position would show the cockpit from outside.
    let cam_transform = match layout {
        Layout::Hud => Transform::IDENTITY,
        Layout::Gallery => {
            Transform::from_xyz(0.0, 0.9, 4.2).looking_at(Vec3::new(0.0, 0.35, 0.0), Vec3::Y)
        }
    };
    commands.spawn((
        Camera3d::default(),
        Camera {
            hdr: true,
            ..default()
        },
        Tonemapping::TonyMcMapface,
        Bloom::NATURAL,
        cam_transform,
    ));

    // Key light plus a cool fill, so the Industrial bezel has something to
    // shade against and the scene is not lit only by the panels themselves.
    commands.spawn((
        DirectionalLight {
            illuminance: 6_000.0,
            shadows_enabled: true,
            ..default()
        },
        Transform::from_xyz(4.0, 8.0, 6.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.spawn((
        PointLight {
            color: Color::srgb(0.4, 0.7, 1.0),
            intensity: 250_000.0,
            range: 30.0,
            ..default()
        },
        Transform::from_xyz(-4.0, 1.5, 3.0),
    ));

    let hull = std_materials.add(StandardMaterial {
        base_color: Color::srgb(0.35, 0.38, 0.45),
        metallic: 0.9,
        perceptual_roughness: 0.25,
        ..default()
    });
    match layout {
        Layout::Gallery => {
            // A solid object among the panels: proves they occupy the same
            // space and are not an overlay.
            commands.spawn((
                Mesh3d(meshes.add(Torus::new(0.28, 0.42))),
                MeshMaterial3d(hull),
                Transform::from_xyz(0.0, 0.35, -0.9),
                Prop,
            ));
            // Floor, to catch the panels' glow and give the scene a ground.
            commands.spawn((
                Mesh3d(meshes.add(Plane3d::default().mesh().size(40.0, 40.0))),
                MeshMaterial3d(std_materials.add(StandardMaterial {
                    base_color: Color::srgb(0.03, 0.035, 0.05),
                    perceptual_roughness: 0.4,
                    metallic: 0.6,
                    ..default()
                })),
                Transform::from_xyz(0.0, -0.75, 0.0),
            ));
        }
        Layout::Hud => {
            // Something to fly toward, far enough ahead that it reads as
            // distance rather than as an object on the dashboard.
            commands.spawn((
                Mesh3d(meshes.add(Torus::new(2.6, 4.0))),
                MeshMaterial3d(hull),
                Transform::from_xyz(1.5, 0.4, -26.0)
                    .with_rotation(Quat::from_rotation_x(1.15) * Quat::from_rotation_z(0.3)),
                Prop,
            ));
            // A scatter of distant marks so the view is not empty black.
            let star = std_materials.add(StandardMaterial {
                base_color: Color::BLACK,
                emissive: LinearRgba::rgb(3.0, 3.4, 4.2),
                unlit: true,
                ..default()
            });
            let star_mesh = meshes.add(Sphere::new(0.16));
            let mut seed = 0x2545_F491_4F6C_DD1Du64;
            for _ in 0..90 {
                // Deterministic scatter: a screenshot must not change between
                // runs, so no wall-clock or thread-local randomness.
                let mut next = || {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    (seed >> 11) as f32 / (1u64 << 53) as f32
                };
                let (a, b, r) = (next(), next(), 22.0 + next() * 26.0);
                let theta = a * std::f32::consts::TAU;
                let phi = (b - 0.5) * 1.2;
                commands.spawn((
                    Mesh3d(star_mesh.clone()),
                    MeshMaterial3d(star.clone()),
                    Transform::from_xyz(
                        theta.sin() * r * 0.6,
                        phi * r * 0.35,
                        -r * (0.6 + 0.4 * theta.cos().abs()),
                    ),
                ));
            }
        }
    }

    // Every `--skin-dir` joins the cycle; the first becomes active so a
    // screenshot run lands on art rather than on a built-in.
    let mut first_art: Option<SkinId> = None;
    for dir in args_all("--skin-dir") {
        match load_skin_dir(&dir, &mut images, &mut registry) {
            Some(id) => {
                first_art.get_or_insert(id);
            }
            None => warn!("could not load skin dir {dir}"),
        }
    }
    let skin = first_art.unwrap_or(SkinId::Builtin(skin_from_args()));
    active.0 = skin;
    let skin_name = registry.name(skin);

    if layout == Layout::Hud {
        hud::build_hud(
            &mut commands,
            &mut meshes,
            &mut panel_materials,
            &mut images,
            &registry,
            skin,
            &blank.0,
        );
        spawn_labels(&mut commands, &skin_name);
        return;
    }

    // Three panels in a shallow arc, the way a cockpit or a shipyard bay
    // would actually place them around the player.
    let centre = spawn_panel(
        &mut commands,
        &mut meshes,
        &mut panel_materials,
        &mut images,
        &PanelDesc::default().size(1.75, 1.15),
        skin,
        blank.0.clone(),
        Transform::from_xyz(0.0, 0.45, 0.0),
    );
    let left = spawn_panel(
        &mut commands,
        &mut meshes,
        &mut panel_materials,
        &mut images,
        &PanelDesc::default().size(1.0, 1.25),
        skin,
        blank.0.clone(),
        Transform::from_xyz(-1.55, 0.45, 0.55).with_rotation(Quat::from_rotation_y(0.42)),
    );
    let right = spawn_panel(
        &mut commands,
        &mut meshes,
        &mut panel_materials,
        &mut images,
        &PanelDesc::default().size(1.0, 1.25),
        skin,
        blank.0.clone(),
        Transform::from_xyz(1.55, 0.45, 0.55).with_rotation(Quat::from_rotation_y(-0.42)),
    );

    let p = registry.params(skin);
    build_catalogue(&mut commands, centre.ui_root, &p);
    build_readout(&mut commands, left.ui_root, &p, "SYSTEMS");
    build_readout(&mut commands, right.ui_root, &p, "CARGO");

    spawn_labels(&mut commands, &skin_name);
}

/// Screen-space help and the active-skin readout. Deliberately NOT panels:
/// they label the demo, they are not part of what is being judged.
fn spawn_labels(commands: &mut Commands, skin_name: &str) {
    commands.spawn((
        Text::new("SPACE cycle skin    ←/→ orbit    ESC quit"),
        TextFont {
            font_size: 15.0,
            ..default()
        },
        TextColor(Color::srgb(0.5, 0.6, 0.7)),
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(16.0),
            left: Val::Px(16.0),
            ..default()
        },
    ));
    commands.spawn((
        Text::new(format!("SKIN  {skin_name}")),
        TextFont {
            font_size: 20.0,
            ..default()
        },
        TextColor(Color::srgb(0.85, 0.92, 1.0)),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(16.0),
            left: Val::Px(16.0),
            ..default()
        },
        SkinLabel,
    ));
}

/// A shipyard-style hull list: the kind of screen this replaces.
fn build_catalogue(commands: &mut Commands, root: Entity, p: &artificer_ui::SkinParams) {
    let rows = [
        ("SPARROW", "40", "615", "29 800"),
        ("HARRIER", "80", "665", "70 600"),
        ("MULE", "480", "935", "191 000"),
        ("ATLAS", "640", "1145", "496 400"),
    ];
    commands.entity(root).with_children(|panel| {
        panel
            .spawn(Node {
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(Val::Px(38.0)),
                row_gap: Val::Px(14.0),
                width: Val::Percent(100.0),
                ..default()
            })
            .with_children(|col| {
                col.spawn((
                    Text::new("SHIPYARD"),
                    TextFont {
                        font_size: 40.0,
                        ..default()
                    },
                    TextColor(p.accent),
                ));
                col.spawn((
                    Text::new("MERIDIAN GATE  ·  CORE"),
                    TextFont {
                        font_size: 21.0,
                        ..default()
                    },
                    TextColor(p.dim_text),
                ));
                col.spawn((
                    Node {
                        width: Val::Percent(100.0),
                        height: Val::Px(2.0),
                        margin: UiRect::vertical(Val::Px(8.0)),
                        ..default()
                    },
                    BackgroundColor(p.accent),
                ));
                for (i, (name, hold, hp, price)) in rows.iter().enumerate() {
                    let selected = i == 2;
                    col.spawn((
                        Node {
                            flex_direction: FlexDirection::Row,
                            justify_content: JustifyContent::SpaceBetween,
                            width: Val::Percent(100.0),
                            padding: UiRect::axes(Val::Px(10.0), Val::Px(4.0)),
                            ..default()
                        },
                        BackgroundColor(if selected {
                            p.accent.with_alpha(0.16)
                        } else {
                            Color::NONE
                        }),
                    ))
                    .with_children(|row| {
                        row.spawn((
                            Text::new(*name),
                            TextFont {
                                font_size: 23.0,
                                ..default()
                            },
                            TextColor(if selected { p.accent } else { p.text }),
                        ));
                        row.spawn((
                            Text::new(format!("{hold} m³    {hp} hp    {price} cr")),
                            TextFont {
                                font_size: 22.0,
                                ..default()
                            },
                            TextColor(p.dim_text),
                        ));
                    });
                }
            });
    });
}

/// A gauge-style side panel.
fn build_readout(commands: &mut Commands, root: Entity, p: &artificer_ui::SkinParams, title: &str) {
    let bars = [
        ("HULL", 0.86),
        ("SHIELD", 0.62),
        ("POWER", 0.94),
        ("FUEL", 0.41),
    ];
    let title = title.to_string();
    commands.entity(root).with_children(|panel| {
        panel
            .spawn(Node {
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(Val::Px(34.0)),
                row_gap: Val::Px(18.0),
                width: Val::Percent(100.0),
                ..default()
            })
            .with_children(|col| {
                col.spawn((
                    Text::new(title),
                    TextFont {
                        font_size: 34.0,
                        ..default()
                    },
                    TextColor(p.accent),
                ));
                for (label, fill) in bars {
                    col.spawn((
                        Text::new(label),
                        TextFont {
                            font_size: 19.0,
                            ..default()
                        },
                        TextColor(p.dim_text),
                    ));
                    // Bar track + fill: geometry, not characters.
                    col.spawn((
                        Node {
                            width: Val::Percent(100.0),
                            height: Val::Px(14.0),
                            ..default()
                        },
                        BackgroundColor(p.dim_text.with_alpha(0.20)),
                    ))
                    .with_children(|track| {
                        track.spawn((
                            Node {
                                width: Val::Percent(fill * 100.0),
                                height: Val::Percent(100.0),
                                ..default()
                            },
                            BackgroundColor(p.accent),
                        ));
                    });
                }
            });
    });
}

fn cycle_skin(
    keys: Res<ButtonInput<KeyCode>>,
    registry: Res<SkinRegistry>,
    mut active: ResMut<ActiveSkin>,
    mut label: Query<&mut Text, With<SkinLabel>>,
) {
    if keys.just_pressed(KeyCode::Space) {
        active.0 = registry.next(active.0);
        let name = registry.name(active.0);
        for mut text in &mut label {
            *text = Text::new(format!("SKIN  {name}"));
        }
    }
}

fn orbit_camera(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut orbit: ResMut<Orbit>,
    mut cam: Query<&mut Transform, With<Camera3d>>,
) {
    let mut dir = 0.0;
    if keys.pressed(KeyCode::ArrowLeft) {
        dir -= 1.0;
    }
    if keys.pressed(KeyCode::ArrowRight) {
        dir += 1.0;
    }
    if dir == 0.0 {
        return;
    }
    orbit.angle += dir * time.delta_secs() * 0.8;
    match layout_from_args() {
        // From the pilot's seat you turn your head; you do not orbit yourself.
        Layout::Hud => {
            for mut t in &mut cam {
                *t = Transform::from_rotation(Quat::from_rotation_y(-orbit.angle * 0.35));
            }
        }
        Layout::Gallery => {
            let radius = 4.3;
            for mut t in &mut cam {
                *t = Transform::from_xyz(
                    orbit.angle.sin() * radius,
                    0.9,
                    orbit.angle.cos() * radius,
                )
                .looking_at(Vec3::new(0.0, 0.35, 0.0), Vec3::Y);
            }
        }
    }
}

fn spin_prop(time: Res<Time>, mut props: Query<&mut Transform, With<Prop>>) {
    for mut t in &mut props {
        t.rotate_y(time.delta_secs() * 0.5);
        t.rotate_x(time.delta_secs() * 0.22);
    }
}
