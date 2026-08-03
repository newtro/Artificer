//! Model viewer: look at baked art properly before committing to it.
//!
//! Judging a model from a fixed screenshot is how you ship a hull that looks
//! fine from behind and wrong from every other angle. This puts one asset at
//! a time on screen and lets you go round it -- both axes, freely, at any
//! distance -- and flip between assets without relaunching.
//!
//! Takes a pack path, so no art lives in this repo:
//!
//! ```text
//! model_viewer --pack path/to/ships.apack
//! model_viewer --pack a.apack --pack b.apack     # both, in one cycle
//! model_viewer --pack ships.apack --grid         # all assets side by side
//! ```
//!
//! CONTROLS
//!   drag LMB / arrows  orbit (horizontal and vertical)
//!   wheel / - =        zoom
//!   drag RMB / WASD-QE pan the focus point (Q/E vertical)
//!   `[` `]`            previous / next asset
//!   SPACE              toggle auto-spin
//!   G                  toggle grid (all assets at once)
//!   L                  cycle lighting: neutral / key-only / flat
//!   R                  reset the view
//!   ESC                exit

use artificer_assets::load::{load_pack, LoadedPack};
use artificer_assets::pack::AssetPack;
use artificer_input::{Key, MouseButton};
use artificer_render::{run_app, EngineCtx, GameClient, RenderConfig};
use artificer_scene::{CameraDesc, EnvironmentDesc, LightDesc, NodeId, SceneGraph, TransformDesc};
use glam::Vec3;
use std::sync::{Mutex, OnceLock};

/// Vertical orbit stops just short of the poles. Straight down the Y axis the
/// camera's up vector is parallel to its view direction and `looking_at`
/// degenerates -- the model flips or vanishes at the exact moment you are
/// trying to inspect it from above.
const PITCH_LIMIT: f32 = 1.53; // ~87.7 degrees

/// Paths waiting to be written by the Bevy-side capture system.
///
/// Captures go through the RENDER TARGET, never through the screen. An
/// earlier version of this drove the window manager and grabbed pixels from
/// the desktop; when the raise silently lost a race it photographed the
/// user's browser instead of the game. A framebuffer capture cannot do that
/// no matter what else is on screen or which window has focus.
fn shot_queue() -> &'static Mutex<Vec<String>> {
    static Q: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    Q.get_or_init(|| Mutex::new(Vec::new()))
}

fn request_shot(path: String) {
    if let Ok(mut q) = shot_queue().lock() {
        q.push(path);
    }
}

/// Drains the queue each frame and asks Bevy to save the primary window's
/// framebuffer.
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

/// One asset from a pack, with what the viewer needs to frame it.
struct Entry {
    id: String,
    /// Radius of the bounding sphere. Used for label placement and pan
    /// speed, NOT for framing -- see `fit_distance`.
    radius: f32,
    /// Half-extents of the bounding box. Framing uses these because a
    /// bounding SPHERE around a flat wide hull is mostly empty air: a ship
    /// 47 m across and 12 m tall got a 30 m sphere radius, and the camera
    /// dutifully backed off far enough to fit a 30 m ball, leaving the ship
    /// at a fifth of the frame.
    half: Vec3,
    /// Bounds centre, so an asset authored off-origin still sits in frame.
    centre: Vec3,
    tris: usize,
}

#[derive(Clone, Copy, PartialEq)]
enum Lighting {
    /// Three-point: what the game roughly looks like.
    Neutral,
    /// Single hard key. Unforgiving -- shows every facet and every smoothing
    /// error, which is exactly what you want when judging hard-surface art.
    KeyOnly,
    /// Heavy ambient, no shadows. Shows silhouette and proportion with the
    /// lighting taken out of the argument.
    Flat,
}

impl Lighting {
    fn label(self) -> &'static str {
        match self {
            Lighting::Neutral => "neutral (3-point)",
            Lighting::KeyOnly => "key only (hard)",
            Lighting::Flat => "flat (ambient)",
        }
    }

    fn next(self) -> Self {
        match self {
            Lighting::Neutral => Lighting::KeyOnly,
            Lighting::KeyOnly => Lighting::Flat,
            Lighting::Flat => Lighting::Neutral,
        }
    }
}

struct Viewer {
    packs: Vec<String>,
    loaded: Option<LoadedPack>,
    entries: Vec<Entry>,
    current: usize,
    /// Spawned nodes for whatever is on screen now, cleared on every change.
    shown: Vec<NodeId>,
    lights: Vec<NodeId>,
    camera: Option<NodeId>,

    yaw: f32,
    pitch: f32,
    /// Multiplier on the distance that exactly FITS the subject, so 1.0
    /// fills the frame and 2.0 is twice as far. Expressed this way rather
    /// than in metres so a 4 m fighter and a 50 m freighter frame the same,
    /// and so the number means something independent of the window's aspect.
    zoom: f32,
    focus: Vec3,
    spin: bool,
    grid: bool,
    lighting: Lighting,

    /// `--shot <dir>`: walk every asset, save a PNG of each, exit. Lets the
    /// whole pack be reviewed without anyone driving the window.
    shot_dir: Option<String>,
    shot_wait: u32,
    shot_taken: usize,
    /// A capture has been queued and not yet serviced.
    shot_pending: bool,
    /// Window size, for aspect-correct framing.
    viewport: glam::Vec2,
}

/// Frames to let an asset settle before capturing it. Materials and textures
/// stream in over the first frames; capturing immediately catches an
/// untextured mesh and makes good art look broken.
const SHOT_SETTLE: u32 = 45;

/// Frames to let a queued capture be serviced before touching the scene.
const SHOT_DRAIN: u32 = 10;

/// Values given for a repeated flag.
///
/// A value that itself looks like a flag is refused rather than consumed:
/// `--pack --grid` must not try to open a file called "--grid", and
/// `--shot --grid` must not create a directory called that.
fn args_all(name: &str) -> Vec<String> {
    let argv: Vec<String> = std::env::args().collect();
    let mut out = Vec::new();
    for (i, a) in argv.iter().enumerate() {
        if a != name {
            continue;
        }
        match argv.get(i + 1) {
            Some(v) if !v.starts_with("--") => out.push(v.clone()),
            _ => {
                eprintln!("{name} needs a value");
                std::process::exit(2);
            }
        }
    }
    out
}

impl Viewer {
    fn new(packs: Vec<String>, grid: bool) -> Self {
        Self {
            packs,
            loaded: None,
            entries: Vec::new(),
            current: 0,
            shown: Vec::new(),
            lights: Vec::new(),
            camera: None,
            yaw: 0.6,
            pitch: 0.35,
            zoom: 1.12,
            focus: Vec3::ZERO,
            spin: false,
            grid,
            lighting: Lighting::Neutral,
            shot_dir: None,
            shot_wait: 0,
            shot_taken: 0,
            shot_pending: false,
            viewport: glam::Vec2::new(1280.0, 720.0),
        }
    }

    /// Unit vector from focus toward the camera, from yaw and pitch.
    fn orbit_dir(&self) -> Vec3 {
        Vec3::new(
            self.pitch.cos() * self.yaw.sin(),
            self.pitch.sin(),
            self.pitch.cos() * self.yaw.cos(),
        )
    }

    /// Half-extents of whatever is on screen, for framing.
    fn active_half(&self) -> Vec3 {
        if self.grid {
            let span: f32 = self.entries.iter().map(|e| e.radius * 2.4).sum();
            let tall = self.entries.iter().map(|e| e.half.y).fold(0.5f32, f32::max);
            Vec3::new(span * 0.5, tall, tall)
        } else {
            self.entries
                .get(self.current)
                .map(|e| e.half)
                .unwrap_or(Vec3::ONE)
        }
    }

    /// Radius of whatever is on screen, for label placement and pan speed.
    fn active_radius(&self) -> f32 {
        if self.grid {
            let span: f32 = self.entries.iter().map(|e| e.radius * 2.4).sum();
            (span * 0.5).max(1.0)
        } else {
            self.entries
                .get(self.current)
                .map(|e| e.radius)
                .unwrap_or(1.0)
        }
    }

    fn reset_view(&mut self) {
        self.yaw = 0.6;
        self.pitch = 0.35;
        self.zoom = 1.12;
        self.focus = Vec3::ZERO;
    }

    fn clear_shown(&mut self, scene: &mut SceneGraph) {
        for node in self.shown.drain(..) {
            scene.despawn(node);
        }
    }

    /// Put the current selection (or every asset, in grid mode) in the scene.
    fn show(&mut self, scene: &mut SceneGraph) {
        self.clear_shown(scene);
        let Some(loaded) = self.loaded.as_ref() else {
            return;
        };

        let picks: Vec<usize> = if self.grid {
            (0..self.entries.len()).collect()
        } else {
            vec![self.current]
        };

        // Lay the grid out left to right, each asset given room proportional
        // to its own size so a big hull does not sit inside a small one.
        let mut cursor = 0.0f32;
        let total: f32 = picks
            .iter()
            .filter_map(|i| self.entries.get(*i))
            .map(|e| e.radius * 2.4)
            .sum();
        let mut spawned = Vec::new();
        for i in picks {
            let Some(entry) = self.entries.get(i) else {
                continue;
            };
            let slot = entry.radius * 2.4;
            let x = if self.grid {
                let x = cursor + slot * 0.5 - total * 0.5;
                cursor += slot;
                x
            } else {
                0.0
            };
            // Shift by -centre so an asset authored off-origin still turns
            // about its own middle rather than swinging around the origin.
            let at = TransformDesc::from_translation(Vec3::new(x, 0.0, 0.0) - entry.centre);
            for part in loaded.parts(&entry.id) {
                spawned.push(scene.spawn_mesh(part.mesh, part.material, at));
            }
        }
        self.shown = spawned;
    }

    fn apply_lighting(&mut self, scene: &mut SceneGraph) {
        for node in self.lights.drain(..) {
            scene.despawn(node);
        }
        let r = self.active_radius().max(1.0);
        let (ambient, lamps): (f32, Vec<(Vec3, [f32; 3], f32)>) = match self.lighting {
            Lighting::Neutral => (
                90.0,
                vec![
                    (Vec3::new(0.7, 0.9, 0.6), [1.0, 0.97, 0.92], 5_000.0),
                    (Vec3::new(-0.8, 0.3, -0.5), [0.75, 0.82, 1.0], 2_200.0),
                    (Vec3::new(0.0, -0.5, -0.9), [1.0, 1.0, 1.0], 1_400.0),
                ],
            ),
            Lighting::KeyOnly => (
                12.0,
                vec![(Vec3::new(0.6, 0.8, 0.4), [1.0, 0.98, 0.94], 9_000.0)],
            ),
            Lighting::Flat => (
                420.0,
                vec![(Vec3::new(0.3, 1.0, 0.2), [1.0, 1.0, 1.0], 900.0)],
            ),
        };

        scene.set_environment(EnvironmentDesc {
            clear_color: [0.04, 0.045, 0.06, 1.0],
            ambient_color: [0.62, 0.68, 0.82],
            ambient_brightness: ambient,
        });

        for (i, (dir, color, lux)) in lamps.into_iter().enumerate() {
            let shadow_caster = i == 0 && !matches!(self.lighting, Lighting::Flat);
            let node = scene.spawn_light(
                LightDesc::Directional {
                    color,
                    illuminance: lux,
                    // Only the key casts. Three shadow-casting directionals
                    // means three shadow passes and cross-hatched shadows
                    // from fill lights, which is not what fill light is for.
                    shadows: shadow_caster,
                },
                TransformDesc::looking_at(dir.normalize() * r * 4.0, Vec3::ZERO, Vec3::Y),
            );
            self.lights.push(node);
        }
    }

    fn place_camera(&mut self, scene: &mut SceneGraph) {
        // Fit the bounding BOX as the camera actually sees it, not a sphere
        // around it. Projecting the box's half-extents onto the camera's own
        // right/up axes gives the true on-screen half-width and half-height,
        // so a flat wide hull fills the frame instead of being framed for the
        // empty sphere that encloses it.
        let vfov = CameraDesc::default().fov_y_degrees.to_radians().max(0.2);
        let aspect = (self.viewport.x / self.viewport.y.max(1.0)).max(0.2);
        let hfov = 2.0 * ((vfov * 0.5).tan() * aspect).atan();

        let fwd = self.orbit_dir();
        let right = fwd.cross(Vec3::Y).normalize_or_zero();
        let up = right.cross(fwd).normalize_or_zero();
        let half = self.active_half();
        let proj =
            |axis: Vec3| half.x * axis.x.abs() + half.y * axis.y.abs() + half.z * axis.z.abs();
        let need_w = proj(right) / (hfov * 0.5).tan().max(0.05);
        let need_h = proj(up) / (vfov * 0.5).tan().max(0.05);
        // Plus the box's own depth toward the camera, or a deep hull viewed
        // nose-on pokes through the near side of the frame.
        let fit = need_w.max(need_h) + proj(fwd);
        let dist = (fit * self.zoom).max(0.35);
        let eye = self.focus + self.orbit_dir() * dist;
        let at = TransformDesc::looking_at(eye, self.focus, Vec3::Y);
        match self.camera {
            Some(cam) => scene.set_transform(cam, at),
            None => {
                let cam = scene.spawn_camera(CameraDesc::default(), at);
                scene.set_active_camera(cam);
                self.camera = Some(cam);
            }
        }
    }
}

impl GameClient for Viewer {
    fn register_bevy(&self, app: &mut bevy::prelude::App) {
        app.add_systems(bevy::prelude::Update, take_queued_shots);
    }

    fn setup(&mut self, ctx: &mut EngineCtx) {
        let mut merged: Option<LoadedPack> = None;
        let mut entries = Vec::new();

        for path in &self.packs {
            let bytes = match std::fs::read(path) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("could not read {path}: {e}");
                    continue;
                }
            };
            let pack = match AssetPack::from_postcard_validated(&bytes) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("{path} is not a usable pack: {e}");
                    continue;
                }
            };
            for asset in &pack.assets {
                let r = &asset.record;
                let min = Vec3::from_array(r.bounds_min);
                let max = Vec3::from_array(r.bounds_max);
                entries.push(Entry {
                    id: r.id.clone(),
                    radius: ((max - min).length() * 0.5).max(0.05),
                    half: ((max - min) * 0.5).max(Vec3::splat(0.02)),
                    centre: (min + max) * 0.5,
                    tris: asset.mesh.triangle_count(),
                });
            }
            let loaded = load_pack(ctx.scene, &pack);
            println!("{path}: {} assets", pack.assets.len());
            match merged.as_mut() {
                Some(all) => {
                    for id in all.merge(loaded) {
                        // Two packs claiming one id is a content bug, and a
                        // silent one shows the wrong ship under the right
                        // name -- exactly the failure a viewer exists to
                        // catch.
                        eprintln!("WARNING: '{id}' in {path} overrides an earlier pack");
                    }
                }
                None => merged = Some(loaded),
            }
        }

        // The merged pack keeps ONE asset per id (last pack wins), so the
        // list must too. Keeping a row per source asset put duplicates in the
        // cycle that both drew the winning mesh while reporting the loser's
        // bounds and triangle count.
        entries.reverse();
        let mut seen = std::collections::HashSet::new();
        entries.retain(|e: &Entry| seen.insert(e.id.clone()));
        entries.sort_by(|a, b| a.id.cmp(&b.id));

        if entries.is_empty() {
            // Opening an empty window and waiting is worse than failing: it
            // reads as broken art rather than a bad path.
            eprintln!("no assets loaded from {:?} -- nothing to view", self.packs);
            std::process::exit(1);
        }
        self.entries = entries;
        self.loaded = merged;

        // A ground plane would fight the art for attention, and a model
        // viewer is not a diorama. The only thing on screen is the asset.
        self.viewport = ctx.window_size;
        self.apply_lighting(ctx.scene);
        self.show(ctx.scene);
        self.place_camera(ctx.scene);
    }

    fn update(&mut self, ctx: &mut EngineCtx) {
        self.viewport = ctx.window_size;
        let dt = ctx.dt;
        let mut view_changed = false;
        let mut content_changed = false;

        // --- batch capture: one PNG per asset, then quit --------------------
        if let Some(dir) = self.shot_dir.clone() {
            // Caption the subject IN the image. A folder of unlabelled
            // renders is only as trustworthy as the claim that file N holds
            // asset N -- and an off-by-one in the capture loop had already
            // shifted every image by one ship once. Burned into the frame,
            // the mapping is checkable instead of asserted.
            if let Some(entry) = self.entries.get(self.shot_taken) {
                let r = entry.radius;
                ctx.labels
                    .push(&entry.id, self.focus + Vec3::Y * r * 1.2, [1.0; 4], 24.0);
                ctx.labels.push(
                    format!("{} tris   {:.1} m across", entry.tris, r * 2.0),
                    self.focus - Vec3::Y * r * 1.15,
                    [0.62, 0.72, 0.85, 1.0],
                    16.0,
                );
            }
            if self.shot_taken >= self.entries.len() {
                // A few frames of grace so the last save lands on disk before
                // the process goes away.
                self.shot_wait += 1;
                if self.shot_wait > 30 {
                    ctx.request_exit();
                }
                return;
            }
            // Re-place every frame. `setup` runs before the window has a real
            // size, so the aspect ratio it framed against was a placeholder --
            // which pushed the camera far enough back that a single-asset pack
            // rendered its ship as a speck. Multi-asset packs hid it, because
            // swapping models re-placed the camera once a true size existed.
            self.place_camera(ctx.scene);

            self.shot_wait += 1;
            if !self.shot_pending && self.shot_wait >= SHOT_SETTLE {
                let id = &self.entries[self.shot_taken].id;
                let safe: String = id
                    .chars()
                    .map(|c| if c.is_alphanumeric() { c } else { '_' })
                    .collect();
                request_shot(format!("{dir}/{:02}_{safe}.png", self.shot_taken + 1));
                self.shot_pending = true;
                self.shot_wait = 0;
            } else if self.shot_pending && self.shot_wait >= SHOT_DRAIN {
                // Swap only AFTER the capture has been serviced. Requesting a
                // shot and changing the model in the same frame meant the
                // capture system ran against the NEW model -- every image was
                // of the next ship along, and the last two came out identical.
                self.shot_pending = false;
                self.shot_taken += 1;
                self.shot_wait = 0;
                if self.shot_taken < self.entries.len() {
                    self.current = self.shot_taken;
                    self.show(ctx.scene);
                    self.apply_lighting(ctx.scene);
                    self.place_camera(ctx.scene);
                }
            }
            return;
        }

        // --- orbit: mouse drag, and arrows for precision -------------------
        if ctx.input.mouse_is_pressed(MouseButton::Left) {
            let d = ctx.input.mouse_delta;
            self.yaw -= d.x * 0.006;
            self.pitch = (self.pitch + d.y * 0.006).clamp(-PITCH_LIMIT, PITCH_LIMIT);
            view_changed = true;
        }
        let kx = ctx.input.axis(Key::Right, Key::Left);
        let ky = ctx.input.axis(Key::Up, Key::Down);
        if kx != 0.0 || ky != 0.0 {
            self.yaw -= kx * dt * 1.6;
            self.pitch = (self.pitch + ky * dt * 1.6).clamp(-PITCH_LIMIT, PITCH_LIMIT);
            view_changed = true;
        }

        // --- zoom ----------------------------------------------------------
        let wheel = ctx.input.wheel_delta;
        let keyzoom = ctx.input.axis(Key::Minus, Key::Equals) * dt * 2.0;
        if wheel != 0.0 || keyzoom != 0.0 {
            // Multiplicative: one wheel notch moves the same PROPORTION at
            // every distance, so you can close in on a detail without the
            // last notch slamming you through the model.
            self.zoom = (self.zoom * (1.0 - wheel * 0.12 + keyzoom)).clamp(0.25, 40.0);
            view_changed = true;
        }

        // --- pan: right-drag, or WASD --------------------------------------
        let mut pan = Vec3::ZERO;
        if ctx.input.mouse_is_pressed(MouseButton::Right) {
            let d = ctx.input.mouse_delta;
            pan += Vec3::new(-d.x, d.y, 0.0) * 0.0016;
        }
        let px = ctx.input.axis(Key::D, Key::A);
        // Q/E, not R/F: R is reset, and sharing it meant the first press
        // to nudge the model upward threw the whole view away instead.
        let py = ctx.input.axis(Key::E, Key::Q);
        let pz = ctx.input.axis(Key::S, Key::W);
        if px != 0.0 || py != 0.0 || pz != 0.0 {
            pan += Vec3::new(px, py, pz) * dt * 0.9;
        }
        if pan != Vec3::ZERO {
            // Pan in the CAMERA's frame, so dragging right always moves the
            // model right on screen no matter where you have orbited to.
            let fwd = self.orbit_dir();
            let right = fwd.cross(Vec3::Y).normalize_or_zero();
            let up = right.cross(fwd).normalize_or_zero();
            self.focus += (right * pan.x + up * pan.y + fwd * pan.z) * self.active_radius();
            view_changed = true;
        }

        // --- selection and modes -------------------------------------------
        if !self.entries.is_empty() {
            let step = i32::from(ctx.input.just_pressed(Key::BracketRight))
                - i32::from(ctx.input.just_pressed(Key::BracketLeft));
            if step != 0 {
                let n = self.entries.len() as i32;
                self.current = (((self.current as i32 + step) % n + n) % n) as usize;
                content_changed = true;
            }
        }
        if ctx.input.just_pressed(Key::G) {
            self.grid = !self.grid;
            content_changed = true;
        }
        if ctx.input.just_pressed(Key::Space) {
            self.spin = !self.spin;
        }
        if ctx.input.just_pressed(Key::L) {
            self.lighting = self.lighting.next();
            self.apply_lighting(ctx.scene);
        }
        if ctx.input.just_pressed(Key::R) {
            self.reset_view();
            view_changed = true;
        }
        if ctx.input.just_pressed(Key::Escape) {
            ctx.request_exit();
        }

        if self.spin {
            self.yaw += dt * 0.5;
            view_changed = true;
        }

        if content_changed {
            self.show(ctx.scene);
            // Relight: the lamps are placed relative to the subject's size,
            // and a grid of ten hulls needs them much further out than one.
            self.apply_lighting(ctx.scene);
        }
        if content_changed || view_changed || self.camera.is_none() {
            self.place_camera(ctx.scene);
        }

        // --- readout --------------------------------------------------------
        let (name, tris) = if self.grid {
            (
                format!("ALL {} assets", self.entries.len()),
                self.entries.iter().map(|e| e.tris).sum::<usize>(),
            )
        } else {
            match self.entries.get(self.current) {
                Some(e) => (
                    format!("[{}/{}]  {}", self.current + 1, self.entries.len(), e.id),
                    e.tris,
                ),
                None => ("no assets".to_string(), 0),
            }
        };
        // `HudBoard` is a value bag the GAME's UI layer reads; the engine
        // draws nothing from it on its own, and a viewer with no idea which
        // asset is on screen is useless. World labels are projected through
        // the active camera by the engine, so they need no UI layer at all.
        //
        // Pinned above and below the subject rather than to screen corners,
        // because that is what a world label can do -- and it keeps the
        // caption attached to the thing it describes while you orbit.
        let r = self.active_radius();
        ctx.hud.set("asset", &name);
        ctx.hud.set("tris", format!("{tris} tris"));
        ctx.labels
            .push(&name, self.focus + Vec3::Y * r * 1.25, [1.0; 4], 22.0);
        ctx.labels.push(
            format!(
                "{tris} tris   {:.1} m across   light: {}",
                r * 2.0,
                self.lighting.label()
            ),
            self.focus - Vec3::Y * r * 1.1,
            [0.62, 0.72, 0.85, 1.0],
            15.0,
        );
        ctx.labels.push(
            "LMB orbit   RMB pan   wheel zoom   [ ] asset   G grid   L light   SPACE spin   R reset",
            self.focus - Vec3::Y * r * 1.45,
            [0.45, 0.5, 0.6, 1.0],
            13.0,
        );
    }
}

fn main() {
    let packs = args_all("--pack");
    if packs.is_empty() {
        eprintln!(
            "model_viewer --pack <file.apack> [--pack <more.apack>] [--grid]\n\
             \n\
             Takes baked packs so no art has to live in this repo."
        );
        std::process::exit(2);
    }
    let grid = std::env::args().any(|a| a == "--grid");
    let mut viewer = Viewer::new(packs, grid);
    if let Some(dir) = args_all("--shot").into_iter().next() {
        if let Err(e) = std::fs::create_dir_all(&dir) {
            eprintln!("could not create {dir}: {e}");
            std::process::exit(1);
        }
        viewer.shot_dir = Some(dir);
    }
    run_app(
        RenderConfig {
            title: "artificer model viewer".to_string(),
            ..Default::default()
        },
        viewer,
    );
}
