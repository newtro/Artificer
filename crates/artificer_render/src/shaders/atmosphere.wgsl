// Single-scattering planetary atmosphere shell (Rayleigh + Mie).
//
// Runs on an additive shell mesh around the planet. The mesh only decides
// WHICH pixels run this shader; the light transport is analytic ray-sphere
// work against the radii in the uniform. Planet centre comes from the mesh's
// own world matrix, so the shell follows its node wherever the scene puts it.
//
// Technique lineage: O'Neil (GPU Gems 2 ch.16) via wwwtyro/glsl-atmosphere
// (public domain / Unlicense), re-derived in WGSL for Bevy's PBR bindings.
// Camera is assumed OUTSIDE the shell (orbital scenery, per ADR-0004).

#import bevy_pbr::forward_io::VertexOutput
#import bevy_pbr::mesh_view_bindings::view
#import bevy_pbr::mesh_functions::get_world_from_local

struct AtmosphereUniform {
    // xyz = sun position (world), w = scattered-light intensity.
    sun: vec4<f32>,
    // x = planet radius, y = atmosphere radius.
    radii: vec4<f32>,
    // xyz = Rayleigh scattering coefficients, w = Rayleigh scale height.
    rayleigh: vec4<f32>,
    // x = Mie coefficient, y = Mie scale height, z = Mie g.
    mie: vec4<f32>,
};

@group(2) @binding(0) var<uniform> atmo: AtmosphereUniform;

const PI: f32 = 3.14159265358979;
// 12x5 samples: the shell covers a small part of the screen and additive
// banding hides under the planet's own shading, so this is enough. Raise
// PRIMARY first if a thick atmosphere ever bands visibly.
const PRIMARY_STEPS: i32 = 12;
const LIGHT_STEPS: i32 = 5;

// Distances along `dir` where the ray enters/exits the sphere.
// A miss returns (1, -1) so `near > far` is the miss test.
fn ray_sphere(origin: vec3<f32>, dir: vec3<f32>, center: vec3<f32>, radius: f32) -> vec2<f32> {
    let oc = origin - center;
    let b = dot(oc, dir);
    let c = dot(oc, oc) - radius * radius;
    let disc = b * b - c;
    if disc < 0.0 {
        return vec2<f32>(1.0, -1.0);
    }
    let s = sqrt(disc);
    return vec2<f32>(-b - s, -b + s);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    // Planet centre = translation column of the mesh's world matrix. The
    // shell mesh is authored around its node origin, same as the planet.
    let world_from_local = get_world_from_local(in.instance_index);
    let center = world_from_local[3].xyz;

    let planet_r = atmo.radii.x;
    let atmos_r = atmo.radii.y;
    let sun_dir = normalize(atmo.sun.xyz - center);
    let intensity = atmo.sun.w;
    let beta_r = atmo.rayleigh.xyz;
    let h_r = max(atmo.rayleigh.w, 1e-4);
    let beta_m = vec3<f32>(atmo.mie.x);
    let h_m = max(atmo.mie.y, 1e-4);
    let g = atmo.mie.z;

    let cam = view.world_position;
    let dir = normalize(in.world_position.xyz - cam);

    // March between entering the shell and leaving it (or hitting ground).
    let shell = ray_sphere(cam, dir, center, atmos_r);
    if shell.x > shell.y {
        return vec4<f32>(0.0);
    }
    let t0 = max(shell.x, 0.0);
    var t1 = shell.y;
    let ground = ray_sphere(cam, dir, center, planet_r);
    if ground.x < ground.y && ground.x > 0.0 {
        t1 = min(t1, ground.x);
    }
    if t1 <= t0 {
        return vec4<f32>(0.0);
    }

    let step_len = (t1 - t0) / f32(PRIMARY_STEPS);
    var sum_r = vec3<f32>(0.0);
    var sum_m = vec3<f32>(0.0);
    var view_od_r = 0.0;
    var view_od_m = 0.0;

    for (var i = 0; i < PRIMARY_STEPS; i = i + 1) {
        let pos = cam + dir * (t0 + (f32(i) + 0.5) * step_len);
        let h = length(pos - center) - planet_r;
        let d_r = exp(-h / h_r) * step_len;
        let d_m = exp(-h / h_m) * step_len;
        view_od_r = view_od_r + d_r;
        view_od_m = view_od_m + d_m;

        // Planet self-shadow: ANALYTIC, not sampled — five march samples
        // can straddle the solid planet and leak sunlight onto the night
        // side of the shell. The exact intersection draws the terminator.
        let ground_block = ray_sphere(pos, sun_dir, center, planet_r);
        let lit = !(ground_block.x < ground_block.y && ground_block.x > 0.0);
        if lit {
            // Optical depth from this sample to the top of the shell.
            let to_sun = ray_sphere(pos, sun_dir, center, atmos_r);
            let lstep = to_sun.y / f32(LIGHT_STEPS);
            var sun_od_r = 0.0;
            var sun_od_m = 0.0;
            for (var j = 0; j < LIGHT_STEPS; j = j + 1) {
                let lpos = pos + sun_dir * ((f32(j) + 0.5) * lstep);
                let lh = max(length(lpos - center) - planet_r, 0.0);
                sun_od_r = sun_od_r + exp(-lh / h_r) * lstep;
                sun_od_m = sun_od_m + exp(-lh / h_m) * lstep;
            }
            // 1.1: Mie extinction slightly exceeds scattering (absorption).
            let tau = beta_r * (view_od_r + sun_od_r) + beta_m * 1.1 * (view_od_m + sun_od_m);
            let attenuation = exp(-tau);
            sum_r = sum_r + d_r * attenuation;
            sum_m = sum_m + d_m * attenuation;
        }
    }

    let mu = dot(dir, sun_dir);
    let mumu = mu * mu;
    let gg = g * g;
    let phase_r = 3.0 / (16.0 * PI) * (1.0 + mumu);
    let phase_m = 3.0 / (8.0 * PI) * ((1.0 - gg) * (mumu + 1.0))
        / (pow(1.0 + gg - 2.0 * mu * g, 1.5) * (2.0 + gg));

    let color = intensity * (phase_r * beta_r * sum_r + phase_m * beta_m * sum_m);
    return vec4<f32>(color, 1.0);
}
