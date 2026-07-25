//! Bevy render adapter: runs the windowed app loop, mirrors the
//! renderer-neutral [`aether_scene::SceneGraph`] into Bevy entities, and
//! feeds platform input into [`aether_input::InputState`].
//!
//! Games implement [`GameClient`] and call [`run_app`]. Game *client* crates
//! may additionally use the sanctioned Bevy extension surface re-exported
//! here as [`bevy`] (see engine ADR-0002) for custom materials and UI.
//! Domain/protocol/server crates must never depend on this crate.

mod convert;
mod keymap;
mod systems;

pub use bevy;

use aether_input::InputState;
use aether_scene::SceneGraph;
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
}

impl Default for RenderConfig {
    fn default() -> Self {
        Self {
            title: "aether".to_string(),
            width: 1280.0,
            height: 720.0,
            vsync: true,
            canvas: None,
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
    pub(crate) exit_requested: bool,
}

impl EngineCtx<'_> {
    pub fn request_exit(&mut self) {
        self.exit_requested = true;
    }
}

/// The game's hook into the engine frame loop.
pub trait GameClient: Send + Sync + 'static {
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

#[derive(Resource)]
pub(crate) struct GameRes(pub Box<dyn GameClient>);

#[derive(Resource, Default)]
pub(crate) struct FrameInfo {
    pub window_size: Vec2,
}

/// Build and run the windowed application. Blocks until exit.
pub fn run_app(config: RenderConfig, game: impl GameClient) {
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
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
    }));

    game.register_bevy(&mut app);

    app.insert_resource(SceneRes(SceneGraph::new()))
        .insert_resource(InputRes(InputState::new()))
        .insert_resource(HudBoard::default())
        .insert_resource(FrameInfo::default())
        .insert_resource(GameRes(Box::new(game)))
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
            )
                .chain(),
        );

    app.run();
}
