//! Cockpit HUD mock built from instruments, not menus.
//!
//! The first version of this was five text panels arranged in a circle, which
//! is a menu wearing a HUD's clothes. Real space-sim cockpits are drawn, not
//! typed:
//!
//! - a **radar plane** whose contacts sit on vertical stalks, so a glance
//!   tells you what is above and below you (Elite's best idea);
//! - a **segmented throttle** with a set-point pin and an optimal-manoeuvring
//!   band, so commanded speed and actual speed can visibly disagree;
//! - **shield quadrants** as four independent arcs, so damage has a bearing;
//! - **arc gauges** for readings you check by needle angle, not by digit;
//! - a **pitch ladder** and a **bracket reticle** in the middle of the view.
//!
//! Text survives only where a number genuinely is the answer — credits,
//! cargo, the name of the system you are in.

use artificer_ui::{
    instrument_quad, spawn_panel, Contact, ContactKind, InstrumentKind, InstrumentMaterial,
    PanelDesc, PanelMaterial, SkinId, SkinParams, SkinRegistry,
};
use bevy::prelude::*;

/// Marks instruments the demo animates, so the mock reads as live
/// instrumentation rather than a still frame.
#[derive(Component)]
pub struct Animated {
    pub kind: InstrumentKind,
}

#[allow(clippy::too_many_arguments)]
pub fn build_hud(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    panel_materials: &mut Assets<PanelMaterial>,
    instrument_materials: &mut Assets<InstrumentMaterial>,
    images: &mut Assets<Image>,
    registry: &SkinRegistry,
    skin: SkinId,
    blank: &Handle<Image>,
) {
    let p = registry.params(skin);
    // Warning colour is deliberately NOT taken from the skin: red means
    // damage under every skin, and a palette that could recolour it would be
    // a safety bug rather than a style choice.
    let warn = Color::srgb(1.0, 0.42, 0.28);

    let instrument = |commands: &mut Commands,
                      meshes: &mut Assets<Mesh>,
                      mats: &mut Assets<InstrumentMaterial>,
                      kind: InstrumentKind,
                      size: Vec2,
                      transform: Transform,
                      build: &dyn Fn(&mut InstrumentMaterial)| {
        let mut m = InstrumentMaterial::new(kind, p.accent, warn, p.dim_text, size.x / size.y);
        m.set_glow(p.emissive * 0.10);
        build(&mut m);
        commands.spawn((
            Mesh3d(instrument_quad(meshes, size)),
            MeshMaterial3d(mats.add(m)),
            transform,
            Animated { kind },
        ));
    };

    // ---- centre of view: pitch ladder and reticle ----
    instrument(
        commands,
        meshes,
        instrument_materials,
        InstrumentKind::Ladder,
        Vec2::splat(0.95),
        Transform::from_xyz(0.0, 0.0, -1.9),
        &|m| {
            m.set_thickness(0.010);
            m.set_glow(0.05);
            // Well back: the ladder crosses the middle of the view and must
            // not compete with whatever you are aiming at.
            m.set_opacity(0.30);
        },
    );
    instrument(
        commands,
        meshes,
        instrument_materials,
        InstrumentKind::Reticle,
        Vec2::splat(0.30),
        Transform::from_xyz(0.0, 0.0, -1.88),
        &|m| {
            m.set_value(0.35); // brackets part-closed: no firing solution yet
            m.set_thickness(0.030);
        },
    );

    // ---- lower left: radar plane ----
    // Contacts spread above and below on purpose, so the stalks read.
    let contacts = [
        Contact {
            plane: Vec2::new(0.35, 0.20),
            elevation: 0.55,
            kind: ContactKind::Neutral,
        },
        Contact {
            plane: Vec2::new(-0.45, 0.40),
            elevation: -0.62,
            kind: ContactKind::Hostile,
        },
        Contact {
            plane: Vec2::new(0.12, -0.55),
            elevation: 0.18,
            kind: ContactKind::Target,
        },
        Contact {
            plane: Vec2::new(-0.20, -0.18),
            elevation: -0.30,
            kind: ContactKind::Neutral,
        },
        Contact {
            plane: Vec2::new(0.62, -0.30),
            elevation: 0.72,
            kind: ContactKind::Neutral,
        },
    ];
    instrument(
        commands,
        meshes,
        instrument_materials,
        InstrumentKind::Radar,
        Vec2::new(0.64, 0.46),
        Transform::from_xyz(-0.74, -0.42, -1.55).with_rotation(Quat::from_rotation_y(0.34)),
        &move |m| {
            m.set_thickness(0.016);
            m.set_contacts(&contacts);
        },
    );

    // ---- lower right: shield quadrants ----
    instrument(
        commands,
        meshes,
        instrument_materials,
        InstrumentKind::Quadrant,
        Vec2::splat(0.42),
        Transform::from_xyz(0.80, -0.40, -1.55).with_rotation(Quat::from_rotation_y(-0.34)),
        &|m| {
            // Port shield down: one arc in the warning colour tells the story
            // faster than four numbers ever could.
            m.set_quadrants(0.92, 0.68, 0.80, 0.21);
            m.set_thickness(0.055);
        },
    );

    // ---- bottom centre: throttle tape and two arc gauges ----
    instrument(
        commands,
        meshes,
        instrument_materials,
        InstrumentKind::Tape,
        Vec2::new(0.20, 0.52),
        Transform::from_xyz(-0.26, -0.44, -1.5).with_rotation(Quat::from_rotation_x(0.22)),
        &|m| {
            m.set_segments(14.0);
            // Commanded 0.86, actual 0.62: the engines are still spooling.
            m.set_throttle(0.62, (0.40, 0.70), 0.86);
            m.set_thickness(0.030);
        },
    );
    instrument(
        commands,
        meshes,
        instrument_materials,
        InstrumentKind::Arc,
        Vec2::splat(0.32),
        Transform::from_xyz(0.10, -0.46, -1.5).with_rotation(Quat::from_rotation_x(0.22)),
        &|m| {
            m.set_arc(0.60, 0.80, 10.0);
            m.set_value(0.74);
            m.set_thickness(0.026);
        },
    );
    instrument(
        commands,
        meshes,
        instrument_materials,
        InstrumentKind::Arc,
        Vec2::splat(0.28),
        Transform::from_xyz(0.42, -0.50, -1.5).with_rotation(Quat::from_rotation_x(0.22)),
        &|m| {
            m.set_arc(0.60, 0.80, 8.0);
            m.set_value(0.41);
            m.set_thickness(0.024);
        },
    );

    // ---- the two places a number really is the answer ----
    let nav = spawn_panel(
        commands,
        meshes,
        panel_materials,
        images,
        &PanelDesc::default().size(0.52, 0.34).resolution(468, 306),
        skin,
        blank.clone(),
        Transform::from_xyz(0.92, 0.32, -1.75).with_rotation(Quat::from_rotation_y(-0.40)),
    );
    let status = spawn_panel(
        commands,
        meshes,
        panel_materials,
        images,
        &PanelDesc::default().size(0.52, 0.26).resolution(468, 234),
        skin,
        blank.clone(),
        Transform::from_xyz(-0.92, 0.34, -1.75).with_rotation(Quat::from_rotation_y(0.40)),
    );
    nav_readout(commands, nav.ui_root, &p);
    contacts_readout(commands, status.ui_root, &p);
}

fn column(commands: &mut Commands, root: Entity, build: impl FnOnce(&mut ChildSpawnerCommands)) {
    commands.entity(root).with_children(|panel| {
        panel
            .spawn(Node {
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(Val::Px(26.0)),
                width: Val::Percent(100.0),
                ..default()
            })
            .with_children(build);
    });
}

fn heading(col: &mut ChildSpawnerCommands, p: &SkinParams, text: &str) {
    let (text, accent) = (text.to_string(), p.accent);
    col.spawn((
        Text::new(text),
        TextFont {
            font_size: 29.0,
            ..default()
        },
        TextColor(accent),
    ));
    col.spawn((
        Node {
            width: Val::Percent(100.0),
            height: Val::Px(3.0),
            margin: UiRect::vertical(Val::Px(8.0)),
            ..default()
        },
        BackgroundColor(accent),
    ));
}

fn key_value(col: &mut ChildSpawnerCommands, p: &SkinParams, key: &str, value: &str) {
    let (key, value) = (key.to_string(), value.to_string());
    let (dim, text) = (p.dim_text, p.text);
    col.spawn(Node {
        flex_direction: FlexDirection::Row,
        justify_content: JustifyContent::SpaceBetween,
        width: Val::Percent(100.0),
        margin: UiRect::bottom(Val::Px(6.0)),
        ..default()
    })
    .with_children(move |row| {
        row.spawn((
            Text::new(key),
            TextFont {
                font_size: 23.0,
                ..default()
            },
            TextColor(dim),
        ));
        row.spawn((
            Text::new(value),
            TextFont {
                font_size: 23.0,
                ..default()
            },
            TextColor(text),
        ));
    });
}

fn nav_readout(commands: &mut Commands, root: Entity, p: &SkinParams) {
    let p = *p;
    column(commands, root, move |col| {
        heading(col, &p, "MERIDIAN");
        key_value(col, &p, "GATE", "4.2 km");
        key_value(col, &p, "CARGO", "312 / 480");
        key_value(col, &p, "CREDITS", "128 400");
    });
}

fn contacts_readout(commands: &mut Commands, root: Entity, p: &SkinParams) {
    let p = *p;
    column(commands, root, move |col| {
        heading(col, &p, "PATROLLED");
        key_value(col, &p, "CONTACTS", "5");
        key_value(col, &p, "TARGET", "KESTREL");
    });
}

/// Drive the gauges from a clock so the mock reads as live instrumentation.
/// Deterministic in time, so screenshots stay comparable between runs.
pub fn animate_instruments(
    time: Res<Time>,
    mut materials: ResMut<Assets<InstrumentMaterial>>,
    q: Query<(&MeshMaterial3d<InstrumentMaterial>, &Animated)>,
) {
    let t = time.elapsed_secs();
    for (handle, animated) in &q {
        let Some(m) = materials.get_mut(&handle.0) else {
            continue;
        };
        match animated.kind {
            InstrumentKind::Arc => {
                m.set_value(0.5 + 0.35 * (t * 0.7).sin());
            }
            InstrumentKind::Tape => {
                let speed = 0.55 + 0.32 * (t * 0.55).sin();
                m.set_throttle(speed, (0.40, 0.70), 0.86);
            }
            InstrumentKind::Quadrant => {
                // Port shield taking hits and recovering.
                let port = (0.20 + 0.20 * (t * 1.3).sin()).max(0.05);
                m.set_quadrants(0.92, 0.68, 0.80, port);
            }
            InstrumentKind::Reticle => {
                // Brackets close as a firing solution converges.
                m.set_value(0.5 + 0.5 * (t * 0.9).sin());
            }
            InstrumentKind::Ladder => {
                m.set_value(0.05 * (t * 0.35).sin());
            }
            // The radar animates in its own shader (the sweep), and its
            // contacts come from the world rather than from a clock.
            InstrumentKind::Radar => {}
        }
    }
}
