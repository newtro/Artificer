//! Rapier 3D physics adapter.
//!
//! Consumers speak glam + opaque handles; Rapier types never cross this
//! boundary, keeping the physics backend replaceable and usable identically
//! from the headless server, the client's prediction, and tests.

use glam::{Quat, Vec3};
use rapier3d::math::Pose3;
use rapier3d::prelude::*;
use std::borrow::Borrow;
use std::sync::Mutex;

/// Rapier's own glam version differs from the engine's public glam, so all
/// values cross this boundary by component.
type RVec = rapier3d::math::Vec3;
type RRot = rapier3d::math::Rot3;

/// Opaque handle to a rigid body in a [`PhysicsWorld`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BodyHandle(RigidBodyHandle);

/// Parameters for a dynamic (simulated) body.
#[derive(Debug, Clone, Copy)]
pub struct DynamicBodyParams {
    pub linear_damping: f32,
    pub angular_damping: f32,
    /// Continuous collision detection for fast movers (projectiles, ships).
    pub ccd: bool,
}

impl Default for DynamicBodyParams {
    fn default() -> Self {
        Self {
            linear_damping: 0.0,
            angular_damping: 0.0,
            ccd: false,
        }
    }
}

/// Result of a ray query.
#[derive(Debug, Clone, Copy)]
pub struct RayHit {
    pub body: BodyHandle,
    pub distance: f32,
    pub point: Vec3,
    pub user_data: u64,
}

/// A collision begin/end between two bodies.
#[derive(Debug, Clone, Copy)]
pub struct CollisionContact {
    pub a: BodyHandle,
    pub b: BodyHandle,
    pub started: bool,
}

fn to_r(v: Vec3) -> RVec {
    RVec::new(v.x, v.y, v.z)
}

fn from_r(v: impl Borrow<RVec>) -> Vec3 {
    let v = v.borrow();
    Vec3::new(v.x, v.y, v.z)
}

fn to_pose(pos: Vec3, rot: Quat) -> Pose3 {
    Pose3::from_parts(to_r(pos), RRot::from_xyzw(rot.x, rot.y, rot.z, rot.w))
}

fn from_pose(p: impl Borrow<Pose3>) -> (Vec3, Quat) {
    let p = p.borrow();
    (
        from_r(p.translation),
        Quat::from_xyzw(p.rotation.x, p.rotation.y, p.rotation.z, p.rotation.w),
    )
}

/// Collects collision events out of Rapier's callback-based handler.
#[derive(Default)]
struct CollisionCollector {
    events: Mutex<Vec<CollisionEvent>>,
}

impl EventHandler for CollisionCollector {
    fn handle_collision_event(
        &self,
        _bodies: &RigidBodySet,
        _colliders: &ColliderSet,
        event: CollisionEvent,
        _contact_pair: Option<&ContactPair>,
    ) {
        self.events.lock().unwrap().push(event);
    }

    fn handle_contact_force_event(
        &self,
        _dt: f32,
        _bodies: &RigidBodySet,
        _colliders: &ColliderSet,
        _contact_pair: &ContactPair,
        _total_force_magnitude: f32,
    ) {
    }
}

pub struct PhysicsWorld {
    bodies: RigidBodySet,
    colliders: ColliderSet,
    integration: IntegrationParameters,
    pipeline: PhysicsPipeline,
    islands: IslandManager,
    broad_phase: BroadPhaseBvh,
    narrow_phase: NarrowPhase,
    impulse_joints: ImpulseJointSet,
    multibody_joints: MultibodyJointSet,
    ccd_solver: CCDSolver,
    gravity: RVec,
    collector: CollisionCollector,
    pending_contacts: Vec<CollisionContact>,
}

impl Default for PhysicsWorld {
    fn default() -> Self {
        Self::new_zero_gravity()
    }
}

impl PhysicsWorld {
    /// Space default: no gravity.
    pub fn new_zero_gravity() -> Self {
        Self::with_gravity(Vec3::ZERO)
    }

    pub fn with_gravity(gravity: Vec3) -> Self {
        Self {
            bodies: RigidBodySet::new(),
            colliders: ColliderSet::new(),
            integration: IntegrationParameters::default(),
            pipeline: PhysicsPipeline::new(),
            islands: IslandManager::new(),
            broad_phase: BroadPhaseBvh::new(),
            narrow_phase: NarrowPhase::new(),
            impulse_joints: ImpulseJointSet::new(),
            multibody_joints: MultibodyJointSet::new(),
            ccd_solver: CCDSolver::new(),
            gravity: to_r(gravity),
            collector: CollisionCollector::default(),
            pending_contacts: Vec::new(),
        }
    }

    // ----- bodies -----

    pub fn add_dynamic(&mut self, pos: Vec3, rot: Quat, params: DynamicBodyParams) -> BodyHandle {
        let rb = RigidBodyBuilder::dynamic()
            .pose(to_pose(pos, rot))
            .linear_damping(params.linear_damping)
            .angular_damping(params.angular_damping)
            .ccd_enabled(params.ccd)
            // Ships in open space must keep integrating damping/forces.
            .can_sleep(false)
            .build();
        BodyHandle(self.bodies.insert(rb))
    }

    pub fn add_fixed(&mut self, pos: Vec3, rot: Quat) -> BodyHandle {
        let rb = RigidBodyBuilder::fixed().pose(to_pose(pos, rot)).build();
        BodyHandle(self.bodies.insert(rb))
    }

    pub fn remove_body(&mut self, handle: BodyHandle) {
        self.bodies.remove(
            handle.0,
            &mut self.islands,
            &mut self.colliders,
            &mut self.impulse_joints,
            &mut self.multibody_joints,
            true,
        );
    }

    pub fn body_count(&self) -> usize {
        self.bodies.len()
    }

    /// Attach an opaque tag (e.g. a game entity id) to a body.
    pub fn set_user_data(&mut self, handle: BodyHandle, data: u64) {
        if let Some(rb) = self.bodies.get_mut(handle.0) {
            rb.user_data = data as u128;
        }
    }

    pub fn user_data(&self, handle: BodyHandle) -> u64 {
        self.bodies
            .get(handle.0)
            .map(|rb| rb.user_data as u64)
            .unwrap_or(0)
    }

    // ----- colliders -----

    pub fn attach_cuboid(&mut self, body: BodyHandle, half_extents: Vec3, density: f32) {
        let c = ColliderBuilder::cuboid(half_extents.x, half_extents.y, half_extents.z)
            .density(density)
            .active_events(ActiveEvents::COLLISION_EVENTS)
            .build();
        self.colliders
            .insert_with_parent(c, body.0, &mut self.bodies);
    }

    pub fn attach_ball(&mut self, body: BodyHandle, radius: f32, density: f32) {
        let c = ColliderBuilder::ball(radius)
            .density(density)
            .active_events(ActiveEvents::COLLISION_EVENTS)
            .build();
        self.colliders
            .insert_with_parent(c, body.0, &mut self.bodies);
    }

    pub fn attach_capsule_z(
        &mut self,
        body: BodyHandle,
        half_height: f32,
        radius: f32,
        density: f32,
    ) {
        let c = ColliderBuilder::capsule_z(half_height, radius)
            .density(density)
            .active_events(ActiveEvents::COLLISION_EVENTS)
            .build();
        self.colliders
            .insert_with_parent(c, body.0, &mut self.bodies);
    }

    // ----- state access -----

    pub fn pose(&self, handle: BodyHandle) -> Option<(Vec3, Quat)> {
        self.bodies.get(handle.0).map(|rb| from_pose(rb.position()))
    }

    pub fn set_pose(&mut self, handle: BodyHandle, pos: Vec3, rot: Quat) {
        if let Some(rb) = self.bodies.get_mut(handle.0) {
            rb.set_position(to_pose(pos, rot), true);
        }
    }

    pub fn velocity(&self, handle: BodyHandle) -> Option<(Vec3, Vec3)> {
        self.bodies
            .get(handle.0)
            .map(|rb| (from_r(rb.linvel()), from_r(rb.angvel())))
    }

    pub fn set_velocity(&mut self, handle: BodyHandle, linear: Vec3, angular: Vec3) {
        if let Some(rb) = self.bodies.get_mut(handle.0) {
            rb.set_linvel(to_r(linear), true);
            rb.set_angvel(to_r(angular), true);
        }
    }

    pub fn mass(&self, handle: BodyHandle) -> f32 {
        self.bodies.get(handle.0).map(|rb| rb.mass()).unwrap_or(0.0)
    }

    // ----- forces (world-space; call each tick after `reset_forces`) -----

    pub fn reset_forces(&mut self, handle: BodyHandle) {
        if let Some(rb) = self.bodies.get_mut(handle.0) {
            rb.reset_forces(true);
            rb.reset_torques(true);
        }
    }

    pub fn apply_force(&mut self, handle: BodyHandle, force: Vec3) {
        if let Some(rb) = self.bodies.get_mut(handle.0) {
            rb.add_force(to_r(force), true);
        }
    }

    pub fn apply_torque(&mut self, handle: BodyHandle, torque: Vec3) {
        if let Some(rb) = self.bodies.get_mut(handle.0) {
            rb.add_torque(to_r(torque), true);
        }
    }

    pub fn apply_impulse(&mut self, handle: BodyHandle, impulse: Vec3) {
        if let Some(rb) = self.bodies.get_mut(handle.0) {
            rb.apply_impulse(to_r(impulse), true);
        }
    }

    // ----- stepping -----

    /// Advance the simulation by `dt` seconds (call once per fixed tick).
    pub fn step(&mut self, dt: f32) {
        self.integration.dt = dt;
        self.pipeline.step(
            self.gravity,
            &self.integration,
            &mut self.islands,
            &mut self.broad_phase,
            &mut self.narrow_phase,
            &mut self.bodies,
            &mut self.colliders,
            &mut self.impulse_joints,
            &mut self.multibody_joints,
            &mut self.ccd_solver,
            &(),
            &self.collector,
        );
        // Translate collider-level events into body-level contacts.
        let events: Vec<CollisionEvent> = self.collector.events.lock().unwrap().drain(..).collect();
        for ev in events {
            let (h1, h2, started) = match ev {
                CollisionEvent::Started(a, b, _) => (a, b, true),
                CollisionEvent::Stopped(a, b, _) => (a, b, false),
            };
            let body_of = |ch: ColliderHandle, colliders: &ColliderSet| {
                colliders.get(ch).and_then(|c| c.parent()).map(BodyHandle)
            };
            if let (Some(a), Some(b)) = (body_of(h1, &self.colliders), body_of(h2, &self.colliders))
            {
                self.pending_contacts
                    .push(CollisionContact { a, b, started });
            }
        }
    }

    /// Contacts accumulated since the last drain.
    pub fn drain_contacts(&mut self) -> Vec<CollisionContact> {
        std::mem::take(&mut self.pending_contacts)
    }

    // ----- queries -----

    /// Cast a ray; returns the closest hit, optionally excluding one body.
    pub fn cast_ray(
        &self,
        origin: Vec3,
        dir: Vec3,
        max_distance: f32,
        exclude: Option<BodyHandle>,
    ) -> Option<RayHit> {
        let mut filter = QueryFilter::default();
        if let Some(ex) = exclude {
            filter = filter.exclude_rigid_body(ex.0);
        }
        let qp = self.broad_phase.as_query_pipeline(
            self.narrow_phase.query_dispatcher(),
            &self.bodies,
            &self.colliders,
            filter,
        );
        let ray = Ray::new(to_r(origin), to_r(dir.normalize_or_zero()));
        let (collider_handle, toi) = qp.cast_ray(&ray, max_distance, true)?;
        let body = self
            .colliders
            .get(collider_handle)?
            .parent()
            .map(BodyHandle)?;
        let hit_point = origin + dir.normalize_or_zero() * toi;
        Some(RayHit {
            body,
            distance: toi,
            point: hit_point,
            user_data: self.user_data(body),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = 1.0 / 60.0;

    #[test]
    fn impulse_moves_body_linearly_in_zero_g() {
        let mut world = PhysicsWorld::new_zero_gravity();
        let body = world.add_dynamic(Vec3::ZERO, Quat::IDENTITY, DynamicBodyParams::default());
        world.attach_ball(body, 0.5, 1.0);
        world.apply_impulse(body, Vec3::X * world.mass(body)); // 1 m/s
        for _ in 0..60 {
            world.step(DT);
        }
        let (pos, _) = world.pose(body).unwrap();
        assert!(
            (pos.x - 1.0).abs() < 0.05,
            "expected ~1m travel, got {}",
            pos.x
        );
        assert!(pos.y.abs() < 1e-3 && pos.z.abs() < 1e-3);
    }

    #[test]
    fn gravity_pulls_body_down() {
        let mut world = PhysicsWorld::with_gravity(Vec3::new(0.0, -9.81, 0.0));
        let body = world.add_dynamic(Vec3::Y * 10.0, Quat::IDENTITY, DynamicBodyParams::default());
        world.attach_ball(body, 0.5, 1.0);
        for _ in 0..30 {
            world.step(DT);
        }
        let (pos, _) = world.pose(body).unwrap();
        assert!(pos.y < 9.5);
    }

    #[test]
    fn raycast_hits_static_box() {
        let mut world = PhysicsWorld::new_zero_gravity();
        let wall = world.add_fixed(Vec3::new(0.0, 0.0, -10.0), Quat::IDENTITY);
        world.attach_cuboid(wall, Vec3::new(5.0, 5.0, 0.5), 1.0);
        world.set_user_data(wall, 77);
        world.step(DT); // build broad-phase structures
        let hit = world
            .cast_ray(Vec3::ZERO, Vec3::NEG_Z, 100.0, None)
            .expect("ray should hit the wall");
        assert_eq!(hit.body, wall);
        assert_eq!(hit.user_data, 77);
        assert!((hit.distance - 9.5).abs() < 0.1);
    }

    #[test]
    fn collision_events_reported() {
        let mut world = PhysicsWorld::new_zero_gravity();
        let a = world.add_dynamic(
            Vec3::new(-2.0, 0.0, 0.0),
            Quat::IDENTITY,
            DynamicBodyParams::default(),
        );
        world.attach_ball(a, 0.5, 1.0);
        let b = world.add_fixed(Vec3::ZERO, Quat::IDENTITY);
        world.attach_ball(b, 0.5, 1.0);
        world.set_velocity(a, Vec3::X * 5.0, Vec3::ZERO);
        let mut contacts = Vec::new();
        for _ in 0..120 {
            world.step(DT);
            contacts.extend(world.drain_contacts());
        }
        assert!(
            contacts.iter().any(|c| c.started),
            "expected a collision start event"
        );
    }

    #[test]
    fn angular_velocity_persists_without_damping() {
        let mut world = PhysicsWorld::new_zero_gravity();
        let body = world.add_dynamic(Vec3::ZERO, Quat::IDENTITY, DynamicBodyParams::default());
        world.attach_cuboid(body, Vec3::new(2.0, 1.0, 4.0), 100.0);
        world.set_velocity(body, Vec3::ZERO, Vec3::Y * 1.0);
        for _ in 0..60 {
            world.step(DT);
        }
        let (_, ang) = world.velocity(body).unwrap();
        assert!(
            (ang.y - 1.0).abs() < 0.05,
            "angular velocity should persist in vacuum, got {ang}"
        );
        // +Y angular velocity (right-hand rule) must rotate -Z toward -X.
        let (_, rot) = world.pose(body).unwrap();
        let fwd = rot * Vec3::NEG_Z;
        assert!(
            fwd.x < -0.5,
            "after 2s at 1 rad/s about +Y, forward should point toward -X, got {fwd}"
        );
    }

    #[test]
    fn damping_slows_body() {
        let mut world = PhysicsWorld::new_zero_gravity();
        let body = world.add_dynamic(
            Vec3::ZERO,
            Quat::IDENTITY,
            DynamicBodyParams {
                linear_damping: 1.0,
                ..Default::default()
            },
        );
        world.attach_ball(body, 0.5, 1.0);
        world.set_velocity(body, Vec3::X * 10.0, Vec3::ZERO);
        for _ in 0..120 {
            world.step(DT);
        }
        let (lin, _) = world.velocity(body).unwrap();
        assert!(lin.x < 2.0, "damping should bleed speed, got {}", lin.x);
    }
}
