//! Stellar-object generator showcase (ADR-0004): every planet archetype in
//! one frame, plus rings, atmospheres, and a scatter of asteroids.
//!
//! Run it to eyeball the generator; pass `--screenshot out.png` to capture
//! the framebuffer after the scene settles and exit (visual regression /
//! remote verification). Captures go through the render target, never the
//! desktop.
//!
//! ```text
//! cargo run -p sample_planets
//! cargo run -p sample_planets -- --screenshot planets.png
//! ```

use artificer_input::Key;
use artificer_procgen::{asteroid_mesh, generate_planet, presets, spawn_planet, AsteroidSpec};
use artificer_render::{bevy, run_app, EngineCtx, GameClient, RenderConfig};
use artificer_scene::{
    CameraDesc, EnvironmentDesc, LightDesc, MaterialDesc, NodeId, TransformDesc,
};
use glam::{Quat, Vec3};
use std::sync::{Mutex, OnceLock};

/// Where the "sun" sits for lighting + every atmosphere shell.
const SUN_POS: Vec3 = Vec3::new(-5_000.0, 2_200.0, 3_500.0);

/// Framebuffer capture queue (same pattern as the model viewer: captures go
/// through the render target so nothing else on the desktop can leak in).
fn shot_queue() -> &'static Mutex<Vec<String>> {
    static Q: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    Q.get_or_init(|| Mutex::new(Vec::new()))
}

fn take_queued_shots(mut commands: bevy::prelude::Commands) {
    use bevy::render::view::screenshot::{save_to_disk, Screenshot};
    let Ok(mut q) = shot_queue().lock() else {
        return;
    };
    for path in q.drain(..) {
        println!("capturing {path}");
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path));
    }
}

struct PlanetsShowcase {
    /// (root node, spin rate) per planet, for a slow idle rotation.
    spinners: Vec<(NodeId, f32)>,
    screenshot: Option<String>,
    single: Option<presets::Archetype>,
    frames: u32,
}

impl GameClient for PlanetsShowcase {
    fn setup(&mut self, ctx: &mut EngineCtx) {
        ctx.scene.set_environment(EnvironmentDesc {
            clear_color: [0.004, 0.005, 0.012, 1.0],
            ambient_color: [0.5, 0.6, 0.9],
            ambient_brightness: 50.0,
        });

        let light_rot = TransformDesc::looking_at(SUN_POS, Vec3::ZERO, Vec3::Y).rotation;
        ctx.scene.spawn_light(
            LightDesc::Directional {
                color: [1.0, 0.95, 0.85],
                illuminance: 10_000.0,
                shadows: false,
            },
            TransformDesc::from_translation_rotation(SUN_POS, light_rot),
        );

        // Debug close-up: one big planet, dead centre. For shader work.
        if let Some(archetype) = self.single {
            let spec = presets::planet_spec(archetype, 1000, 200.0);
            let planet = generate_planet(&spec);
            spawn_planet(ctx.scene, &planet, TransformDesc::IDENTITY, SUN_POS);
            let camera = ctx.scene.spawn_camera(
                CameraDesc::default(),
                TransformDesc::looking_at(Vec3::new(0.0, 0.0, 700.0), Vec3::ZERO, Vec3::Y),
            );
            ctx.scene.set_active_camera(camera);
            return;
        }

        // 3x3 grid, one archetype each. Rings go where rings look best.
        let radius = 70.0;
        for (i, archetype) in presets::Archetype::ALL.into_iter().enumerate() {
            let col = (i % 3) as f32 - 1.0;
            let row = (i / 3) as f32 - 1.0;
            let position = Vec3::new(col * 240.0, -row * 210.0, 0.0);

            let mut spec = presets::planet_spec(archetype, 1000 + i as u64, radius);
            if matches!(
                archetype,
                presets::Archetype::GasGiant | presets::Archetype::Ice
            ) {
                spec.ring = Some(presets::ring_spec(spec.seed));
            }
            let planet = generate_planet(&spec);
            // Tilt each body a little so rings and caps read in silhouette.
            let tilt = Quat::from_rotation_z(0.35 + i as f32 * 0.07)
                * Quat::from_rotation_x(0.12 * (i as f32 - 4.0));
            let nodes = spawn_planet(
                ctx.scene,
                &planet,
                TransformDesc::from_translation_rotation(position, tilt),
                SUN_POS,
            );
            self.spinners
                .push((nodes.root, 0.02 + 0.01 * (i as f32 % 3.0)));
        }

        // A pinch of asteroid field under the grid.
        for i in 0..6u64 {
            let mesh = ctx.scene.add_mesh(asteroid_mesh(&AsteroidSpec {
                seed: 40 + i,
                radius: 16.0,
                ..Default::default()
            }));
            let x = -320.0 + i as f32 * 128.0;
            ctx.scene.spawn_mesh(
                mesh,
                MaterialDesc {
                    base_color: [0.4, 0.36, 0.32, 1.0],
                    roughness: 0.95,
                    ..Default::default()
                },
                TransformDesc::from_translation_rotation(
                    Vec3::new(x, -330.0, 60.0),
                    Quat::from_euler(glam::EulerRot::XYZ, i as f32, i as f32 * 1.7, 0.3),
                ),
            );
        }

        let camera = ctx.scene.spawn_camera(
            CameraDesc::default(),
            TransformDesc::looking_at(Vec3::new(0.0, 40.0, 760.0), Vec3::ZERO, Vec3::Y),
        );
        ctx.scene.set_active_camera(camera);
    }

    fn update(&mut self, ctx: &mut EngineCtx) {
        self.frames += 1;
        for (node, rate) in &self.spinners {
            if let Some(mut t) = ctx.scene.transform(*node) {
                t.rotation = Quat::from_rotation_y(*rate * ctx.dt) * t.rotation;
                ctx.scene.set_transform(*node, t);
            }
        }

        if ctx.input.just_pressed(Key::F12) {
            if let Ok(mut q) = shot_queue().lock() {
                q.push("planets-shot.png".to_string());
            }
        }
        if let Some(path) = &self.screenshot {
            // Let textures upload and the first frames present before
            // capturing; exit once the observer has had time to write.
            if self.frames == 45 {
                if let Ok(mut q) = shot_queue().lock() {
                    q.push(path.clone());
                }
            }
            if self.frames > 140 {
                ctx.request_exit();
            }
        }
        if ctx.input.just_pressed(Key::Escape) {
            ctx.request_exit();
        }
    }

    fn register_bevy(&self, app: &mut bevy::prelude::App) {
        use bevy::prelude::IntoScheduleConfigs;
        app.add_systems(bevy::prelude::Update, take_queued_shots.into_configs());
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let screenshot = args
        .iter()
        .position(|a| a == "--screenshot")
        .and_then(|i| args.get(i + 1))
        .cloned();
    let single = args.iter().position(|a| a == "--single").map(|i| {
        match args.get(i + 1).map(String::as_str) {
            Some("ocean") => presets::Archetype::Ocean,
            Some("desert") => presets::Archetype::Desert,
            Some("ice") => presets::Archetype::Ice,
            Some("lava") => presets::Archetype::Lava,
            Some("toxic") => presets::Archetype::Toxic,
            Some("barren") => presets::Archetype::Barren,
            Some("gas") => presets::Archetype::GasGiant,
            Some("icegiant") => presets::Archetype::IceGiant,
            _ => presets::Archetype::EarthLike,
        }
    });
    run_app(
        RenderConfig {
            title: "artificer planets sample".to_string(),
            width: 1600.0,
            height: 900.0,
            ..Default::default()
        },
        PlanetsShowcase {
            spinners: Vec::new(),
            screenshot,
            single,
            frames: 0,
        },
    );
}
