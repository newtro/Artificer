//! Cockpit HUD mock: instruments as world-space panels around the viewer.
//!
//! The arrangement is the point. Instruments sit at the edges of vision,
//! angled inward toward the pilot, with the flight view clear through the
//! middle — so the whole HUD parallaxes and catches light like part of the
//! ship instead of being pasted flat on the glass.

use artificer_ui::{spawn_panel, PanelDesc, PanelMaterial, SkinId, SkinParams, SkinRegistry};
use bevy::prelude::*;

/// Build the full HUD in the active skin.
#[allow(clippy::too_many_arguments)]
pub fn build_hud(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    panel_materials: &mut Assets<PanelMaterial>,
    images: &mut Assets<Image>,
    registry: &SkinRegistry,
    skin: SkinId,
    blank: &Handle<Image>,
) {
    let p = registry.params(skin);

    // Left: ship condition, yawed inward to face the pilot.
    let left = panel(
        commands,
        meshes,
        panel_materials,
        images,
        skin,
        blank,
        Vec2::new(0.64, 0.56),
        Transform::from_xyz(-0.97, -0.06, -1.85).with_rotation(Quat::from_rotation_y(0.42)),
    );
    // Right: navigation and holdings.
    let right = panel(
        commands,
        meshes,
        panel_materials,
        images,
        skin,
        blank,
        Vec2::new(0.64, 0.56),
        Transform::from_xyz(0.97, -0.06, -1.85).with_rotation(Quat::from_rotation_y(-0.42)),
    );
    // Bottom: throttle and speed, pitched up toward the eye like a console
    // lip rather than standing vertical.
    let bottom = panel(
        commands,
        meshes,
        panel_materials,
        images,
        skin,
        blank,
        Vec2::new(0.98, 0.30),
        Transform::from_xyz(0.0, -0.55, -1.72).with_rotation(Quat::from_rotation_x(0.46)),
    );
    // Top: contacts and standing, pitched down.
    let top = panel(
        commands,
        meshes,
        panel_materials,
        images,
        skin,
        blank,
        Vec2::new(0.86, 0.21),
        Transform::from_xyz(0.0, 0.60, -1.9).with_rotation(Quat::from_rotation_x(-0.26)),
    );

    condition(commands, left, &p);
    nav(commands, right, &p);
    throttle(commands, bottom, &p);
    contacts(commands, top, &p);
}

#[allow(clippy::too_many_arguments)]
fn panel(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<PanelMaterial>,
    images: &mut Assets<Image>,
    skin: SkinId,
    blank: &Handle<Image>,
    size: Vec2,
    transform: Transform,
) -> Entity {
    spawn_panel(
        commands,
        meshes,
        materials,
        images,
        &PanelDesc::default()
            .size(size.x, size.y)
            // 900 px/m rather than the 512 default: HUD panels are small in
            // world units and sit close to the eye, so they need the density.
            .resolution((size.x * 900.0) as u32, (size.y * 900.0) as u32),
        skin,
        blank.clone(),
        transform,
    )
    .ui_root
}

fn column(commands: &mut Commands, root: Entity, build: impl FnOnce(&mut ChildSpawnerCommands)) {
    commands.entity(root).with_children(|panel| {
        panel
            .spawn(Node {
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(Val::Px(30.0)),
                width: Val::Percent(100.0),
                ..default()
            })
            .with_children(build);
    });
}

fn rule(col: &mut ChildSpawnerCommands, p: &SkinParams) {
    col.spawn((
        Node {
            width: Val::Percent(100.0),
            height: Val::Px(3.0),
            margin: UiRect::vertical(Val::Px(9.0)),
            ..default()
        },
        BackgroundColor(p.accent),
    ));
}

fn heading(col: &mut ChildSpawnerCommands, p: &SkinParams, text: &str) {
    col.spawn((
        Text::new(text.to_string()),
        TextFont {
            font_size: 30.0,
            ..default()
        },
        TextColor(p.accent),
    ));
}

/// A labelled gauge: track and fill, both geometry rather than characters.
fn bar_row(col: &mut ChildSpawnerCommands, p: &SkinParams, label: &str, fill: f32) {
    let (label, pct) = (label.to_string(), format!("{:.0}%", fill * 100.0));
    let (dim, text, accent) = (p.dim_text, p.text, p.accent);
    col.spawn(Node {
        flex_direction: FlexDirection::Row,
        justify_content: JustifyContent::SpaceBetween,
        width: Val::Percent(100.0),
        ..default()
    })
    .with_children(move |row| {
        row.spawn((
            Text::new(label),
            TextFont {
                font_size: 25.0,
                ..default()
            },
            TextColor(dim),
        ));
        row.spawn((
            Text::new(pct),
            TextFont {
                font_size: 25.0,
                ..default()
            },
            TextColor(text),
        ));
    });
    col.spawn((
        Node {
            width: Val::Percent(100.0),
            height: Val::Px(16.0),
            margin: UiRect::bottom(Val::Px(9.0)),
            ..default()
        },
        BackgroundColor(dim.with_alpha(0.22)),
    ))
    .with_children(move |track| {
        track.spawn((
            Node {
                width: Val::Percent(fill * 100.0),
                height: Val::Percent(100.0),
                ..default()
            },
            BackgroundColor(accent),
        ));
    });
}

fn key_value(col: &mut ChildSpawnerCommands, p: &SkinParams, key: &str, value: &str) {
    let (key, value) = (key.to_string(), value.to_string());
    let (dim, text) = (p.dim_text, p.text);
    col.spawn(Node {
        flex_direction: FlexDirection::Row,
        justify_content: JustifyContent::SpaceBetween,
        width: Val::Percent(100.0),
        margin: UiRect::bottom(Val::Px(7.0)),
        ..default()
    })
    .with_children(move |row| {
        row.spawn((
            Text::new(key),
            TextFont {
                font_size: 25.0,
                ..default()
            },
            TextColor(dim),
        ));
        row.spawn((
            Text::new(value),
            TextFont {
                font_size: 25.0,
                ..default()
            },
            TextColor(text),
        ));
    });
}

fn condition(commands: &mut Commands, root: Entity, p: &SkinParams) {
    let p = *p;
    column(commands, root, move |col| {
        heading(col, &p, "CONDITION");
        rule(col, &p);
        bar_row(col, &p, "HULL", 0.86);
        bar_row(col, &p, "SHIELD", 0.62);
        bar_row(col, &p, "POWER", 0.94);
    });
}

fn nav(commands: &mut Commands, root: Entity, p: &SkinParams) {
    let p = *p;
    column(commands, root, move |col| {
        heading(col, &p, "MERIDIAN");
        rule(col, &p);
        key_value(col, &p, "GATE", "4.2 km");
        key_value(col, &p, "CARGO", "312 / 480");
        key_value(col, &p, "CREDITS", "128 400");
        key_value(col, &p, "TARGET", "none");
    });
}

fn throttle(commands: &mut Commands, root: Entity, p: &SkinParams) {
    let p = *p;
    column(commands, root, move |col| {
        let (dim, text, accent) = (p.dim_text, p.text, p.accent);
        col.spawn(Node {
            flex_direction: FlexDirection::Row,
            justify_content: JustifyContent::SpaceBetween,
            align_items: AlignItems::FlexEnd,
            width: Val::Percent(100.0),
            ..default()
        })
        .with_children(move |row| {
            row.spawn((
                Text::new("212"),
                TextFont {
                    font_size: 62.0,
                    ..default()
                },
                TextColor(text),
            ));
            row.spawn((
                Text::new("m/s"),
                TextFont {
                    font_size: 26.0,
                    ..default()
                },
                TextColor(dim),
            ));
            row.spawn((
                Text::new("THR 78%    FUEL 64"),
                TextFont {
                    font_size: 26.0,
                    ..default()
                },
                TextColor(dim),
            ));
        });
        col.spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Px(18.0),
                margin: UiRect::top(Val::Px(10.0)),
                ..default()
            },
            BackgroundColor(dim.with_alpha(0.22)),
        ))
        .with_children(move |track| {
            track.spawn((
                Node {
                    width: Val::Percent(78.0),
                    height: Val::Percent(100.0),
                    ..default()
                },
                BackgroundColor(accent),
            ));
        });
    });
}

fn contacts(commands: &mut Commands, root: Entity, p: &SkinParams) {
    let p = *p;
    column(commands, root, move |col| {
        let (text, accent) = (p.text, p.accent);
        col.spawn(Node {
            flex_direction: FlexDirection::Row,
            justify_content: JustifyContent::SpaceBetween,
            width: Val::Percent(100.0),
            ..default()
        })
        .with_children(move |row| {
            row.spawn((
                Text::new("CONTACTS  3"),
                TextFont {
                    font_size: 27.0,
                    ..default()
                },
                TextColor(text),
            ));
            row.spawn((
                Text::new("PATROLLED SPACE"),
                TextFont {
                    font_size: 27.0,
                    ..default()
                },
                TextColor(accent),
            ));
        });
    });
}
