//! Bevy render adapter: runs the windowed app loop, mirrors the
//! renderer-neutral [`artificer_scene::SceneGraph`] into Bevy entities, and
//! feeds platform input into [`artificer_input::InputState`].
//!
//! Games implement [`GameClient`] and call [`run_app`]. Game *client* crates
//! may additionally use the sanctioned Bevy extension surface re-exported
//! here as [`bevy`] (see engine ADR-0002) for custom materials and UI.
//! Domain/protocol/server crates must never depend on this crate.

mod convert;
mod keymap;
pub mod labels;
mod systems;

pub use bevy;
pub use labels::{WorldLabel, WorldLabels};

/// Scene handles as the renderer sees them.
///
/// Exposed so client crates can build things out of meshes the scene graph
/// already owns -- UI thumbnails of real geometry, for one -- without every
/// game re-uploading its own copy. Read-only by convention: the adapter owns
/// these and rebuilds them from scene commands.
pub use convert::material_from_desc;
pub use systems::AdapterMaps;

use artificer_input::InputState;
use artificer_scene::SceneGraph;
use bevy::prelude::*;
use bevy::window::PresentMode;
use glam::Vec2;

/// Window/runtime configuration for a rendering client.
#[derive(Debug, Clone)]
pub struct RenderConfig {
    pub title: String,
    pub width: f32,
    pub height: f32,
    pub vsync: bool,
    /// CSS selector of the canvas to attach to on web (e.g. "#game-canvas").
    pub canvas: Option<String>,
    /// Where the game's assets live.
    ///
    /// `None` uses Bevy's default, which resolves relative to the executable
    /// — right for a shipped build, wrong for `cargo run` out of a workspace
    /// whose target directory is somewhere else entirely. Games that keep
    /// assets beside their source set this.
    pub assets_dir: Option<String>,
}

impl Default for RenderConfig {
    fn default() -> Self {
        Self {
            title: "artificer".to_string(),
            width: 1280.0,
            height: 720.0,
            vsync: true,
            canvas: None,
            assets_dir: None,
        }
    }
}

/// Per-frame context handed to the game.
pub struct EngineCtx<'a> {
    pub scene: &'a mut SceneGraph,
    pub input: &'a InputState,
    /// Real seconds since last frame (render dt, not fixed dt).
    pub dt: f32,
    /// Real seconds since app start.
    pub elapsed: f32,
    /// Current window size in logical pixels.
    pub window_size: Vec2,
    /// Dynamic values for the game's UI layer (see [`HudBoard`]).
    pub hud: &'a mut HudBoard,
    /// World-space labels to render this frame (cleared each frame).
    pub labels: &'a mut WorldLabels,
    /// Ray through the cursor from the active scene camera, in world space,
    /// as `(origin, unit direction)`.
    ///
    /// `None` when there is no active scene camera or the cursor is outside
    /// the window. This is the primitive 3D picking needs -- pick a gizmo,
    /// aim at a world-space panel (see `artificer_ui::raycast_panel`), click
    /// a thing in the world -- and it has to come from the engine because
    /// only the engine knows which camera is actually drawing the scene.
    pub cursor_ray: Option<(Vec3, Vec3)>,
    pub(crate) exit_requested: bool,
    pub(crate) cursor_grab_request: Option<bool>,
}

impl EngineCtx<'_> {
    /// Where the cursor ray meets a sphere, if it does. Distance along the
    /// ray, so callers can keep the NEAREST hit when several overlap.
    pub fn ray_hits_sphere(&self, centre: Vec3, radius: f32) -> Option<f32> {
        let (origin, dir) = self.cursor_ray?;
        let to_centre = centre - origin;
        let along = to_centre.dot(dir);
        let miss_sq = to_centre.length_squared() - along * along;
        let radius_sq = radius * radius;
        if miss_sq > radius_sq {
            return None;
        }
        // Distance to where the ray ENTERS the sphere, not to the point
        // nearest its centre. Callers keep the nearest hit, and the two
        // orderings disagree whenever the radii differ -- a big far gizmo
        // would win over a small near one.
        let half_chord = (radius_sq - miss_sq).max(0.0).sqrt();
        let entry = along - half_chord;
        let hit = if entry >= 0.0 {
            entry
        } else {
            along + half_chord
        };
        // Behind the camera counts as a miss; otherwise something at your back
        // is pickable through your own head.
        (hit >= 0.0).then_some(hit)
    }

    pub fn request_exit(&mut self) {
        self.exit_requested = true;
    }

    /// Lock + hide the OS cursor (mouse-look flight) or release it (menus,
    /// map screens). Idempotent; the platform window is only touched when
    /// the requested state actually changes.
    pub fn set_cursor_grab(&mut self, grab: bool) {
        self.cursor_grab_request = Some(grab);
    }
}

/// Desired cursor grab state, applied to the primary window on change.
#[derive(Resource, Default, PartialEq)]
pub(crate) struct CursorGrab(pub bool);

/// The game's hook into the engine frame loop. Runs on the main thread as a
/// non-send resource, so games may hold platform handles (sockets, JS
/// objects) without artificial thread-safety requirements.
pub trait GameClient: 'static {
    /// Called once after the renderer is ready.
    fn setup(&mut self, ctx: &mut EngineCtx);

    /// Called every frame before scene sync.
    fn update(&mut self, ctx: &mut EngineCtx);

    /// Sanctioned Bevy extension point (ADR-0002): register custom plugins,
    /// materials, and UI before the app starts.
    fn register_bevy(&self, _app: &mut App) {}
}

/// Generic key-value blackboard the game's frame update writes and the
/// game's UI (registered via [`GameClient::register_bevy`]) reads. Keeps
/// dynamic HUD data flowing without exposing engine internals.
#[derive(Resource, Default)]
pub struct HudBoard(pub std::collections::HashMap<String, String>);

impl HudBoard {
    pub fn set(&mut self, key: &str, value: impl Into<String>) {
        self.0.insert(key.to_string(), value.into());
    }

    pub fn get(&self, key: &str) -> &str {
        self.0.get(key).map(|s| s.as_str()).unwrap_or("")
    }
}

#[derive(Resource)]
pub(crate) struct SceneRes(pub SceneGraph);

#[derive(Resource)]
pub(crate) struct InputRes(pub InputState);

/// Held as a NON-SEND resource: game code always runs on the main thread.
pub(crate) struct GameRes(pub Box<dyn GameClient>);

#[derive(Resource, Default)]
pub(crate) struct FrameInfo {
    pub window_size: Vec2,
}

/// Build and run the windowed application. Blocks until exit.
pub fn run_app(config: RenderConfig, game: impl GameClient) {
    let mut app = App::new();
    let mut plugins = DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: config.title.clone(),
            resolution: (config.width, config.height).into(),
            present_mode: if config.vsync {
                PresentMode::AutoVsync
            } else {
                PresentMode::AutoNoVsync
            },
            canvas: config.canvas.clone(),
            fit_canvas_to_parent: true,
            prevent_default_event_handling: true,
            ..Default::default()
        }),
        ..Default::default()
    });
    if let Some(dir) = &config.assets_dir {
        plugins = plugins.set(bevy::asset::AssetPlugin {
            file_path: dir.clone(),
            ..Default::default()
        });
    }
    app.add_plugins(plugins);

    game.register_bevy(&mut app);

    app.add_plugins(labels::WorldLabelPlugin)
        .insert_resource(SceneRes(SceneGraph::new()))
        .insert_resource(InputRes(InputState::new()))
        .insert_resource(HudBoard::default())
        .insert_resource(FrameInfo::default())
        .insert_resource(CursorGrab::default())
        .insert_non_send_resource(GameRes(Box::new(game)))
        .insert_resource(ClearColor(Color::BLACK))
        .insert_resource(systems::AdapterMaps::default())
        .add_systems(Startup, systems::game_setup)
        .add_systems(
            Update,
            (
                systems::collect_input,
                systems::game_update,
                systems::apply_scene_commands,
                systems::sync_active_camera,
                systems::apply_cursor_grab,
            )
                .chain(),
        );

    app.run();
}
