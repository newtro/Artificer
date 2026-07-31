//! Engine generality sample (plan §9.4): a tiny second "game" built ONLY on
//! public engine APIs. Run windowed (default) or `-- --headless`.
//!
//! Windowed: an emissive cube orbits a point light; press Escape to exit.
//! Headless: a deterministic physics drift scenario with assertions.

use artificer_assets::procmesh;
use artificer_input::Key;
use artificer_physics::{DynamicBodyParams, PhysicsWorld};
use artificer_render::{run_app, EngineCtx, GameClient, RenderConfig};
use artificer_scene::{
    CameraDesc, EnvironmentDesc, LightDesc, MaterialDesc, NodeId, TransformDesc,
};
use artificer_testkit::{run_scenario, Scenario, ScenarioCtx};
use glam::{Quat, Vec3};

/// A body given one impulse drifts predictably in zero-g.
struct DriftScenario {
    world: PhysicsWorld,
    body: Option<artificer_physics::BodyHandle>,
}

impl Scenario for DriftScenario {
    fn name(&self) -> &'static str {
        "minimal-drift"
    }

    fn ticks(&self) -> u64 {
        120
    }

    fn tick_rate(&self) -> f64 {
        60.0
    }

    fn setup(&mut self, _ctx: &mut ScenarioCtx) {
        let body = self
            .world
            .add_dynamic(Vec3::ZERO, Quat::IDENTITY, DynamicBodyParams::default());
        self.world.attach_ball(body, 0.5, 1.0);
        let mass = self.world.mass(body);
        self.world
            .apply_impulse(body, Vec3::new(2.0, 0.0, 0.0) * mass);
        self.body = Some(body);
    }

    fn tick(&mut self, ctx: &mut ScenarioCtx) {
        self.world.step(ctx.dt as f32);
        if let Some((pos, _)) = self.body.and_then(|b| self.world.pose(b)) {
            ctx.record("x", pos.x as f64);
        }
    }

    fn verify(&mut self, ctx: &mut ScenarioCtx) {
        let (pos, _) = self.body.and_then(|b| self.world.pose(b)).unwrap();
        ctx.check_near("drift distance", pos.x as f64, 4.0, 0.1);
        ctx.check(
            "no lateral drift",
            pos.y.abs() < 1e-3 && pos.z.abs() < 1e-3,
            format!("{pos}"),
        );
    }
}

fn run_headless() {
    let mut scenario = DriftScenario {
        world: PhysicsWorld::new_zero_gravity(),
        body: None,
    };
    let report = run_scenario(&mut scenario);
    println!("{}", report.to_json());
    if !report.passed {
        std::process::exit(1);
    }
}

/// Windowed sample game: orbiting emissive cube.
struct MinimalGame {
    cube: Option<NodeId>,
    angle: f32,
}

impl GameClient for MinimalGame {
    fn setup(&mut self, ctx: &mut EngineCtx) {
        ctx.scene.set_environment(EnvironmentDesc {
            clear_color: [0.02, 0.02, 0.04, 1.0],
            ambient_color: [0.6, 0.7, 1.0],
            ambient_brightness: 120.0,
        });

        let cube_mesh = ctx.scene.add_mesh(procmesh::cuboid(1.0, 1.0, 1.0));
        let cube = ctx.scene.spawn_mesh(
            cube_mesh,
            MaterialDesc::glow(0.2, 0.9, 1.0, 4.0),
            TransformDesc::from_translation(Vec3::new(3.0, 0.0, 0.0)),
        );
        self.cube = Some(cube);

        let floor_mesh = ctx.scene.add_mesh(procmesh::quad_xz(20.0, 20.0));
        ctx.scene.spawn_mesh(
            floor_mesh,
            MaterialDesc::color(0.25, 0.25, 0.3),
            TransformDesc::from_translation(Vec3::Y * -2.0),
        );

        ctx.scene.spawn_light(
            LightDesc::Point {
                color: [1.0, 0.9, 0.8],
                intensity: 2_000_000.0,
                range: 60.0,
                shadows: false,
            },
            TransformDesc::from_translation(Vec3::new(0.0, 4.0, 0.0)),
        );

        let camera = ctx.scene.spawn_camera(
            CameraDesc::default(),
            TransformDesc::looking_at(Vec3::new(0.0, 4.0, 10.0), Vec3::ZERO, Vec3::Y),
        );
        ctx.scene.set_active_camera(camera);
    }

    fn update(&mut self, ctx: &mut EngineCtx) {
        self.angle += ctx.dt * 0.8;
        if let Some(cube) = self.cube {
            let pos = Vec3::new(self.angle.cos() * 3.0, 0.0, self.angle.sin() * 3.0);
            let rot = Quat::from_rotation_y(self.angle * 2.0);
            ctx.scene
                .set_transform(cube, TransformDesc::from_translation_rotation(pos, rot));
        }
        if ctx.input.just_pressed(Key::Escape) {
            ctx.request_exit();
        }
    }
}

fn main() {
    let headless = std::env::args().any(|a| a == "--headless");
    if headless {
        run_headless();
    } else {
        run_app(
            RenderConfig {
                title: "artificer minimal sample".to_string(),
                ..Default::default()
            },
            MinimalGame {
                cube: None,
                angle: 0.0,
            },
        );
    }
}
