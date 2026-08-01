// Flight instruments drawn as signed-distance fields on a quad.
//
// One pipeline, several instrument kinds, because a HUD wants a dozen of
// these and a pipeline switch per gauge is a waste. Everything is drawn in
// the quad's UV space so an instrument is just a textured quad the game can
// place, rotate and skin like any other panel.
//
// The vocabulary here is taken from what space sims actually use: a radar
// plane with elevation stalks, a segmented throttle with a set-point pin, arc
// gauges with tick marks, quadrant shield rings, and a bracket reticle.

#import bevy_pbr::forward_io::VertexOutput
#import bevy_pbr::mesh_view_bindings::globals

const KIND_ARC: u32 = 0u;
const KIND_RADAR: u32 = 1u;
const KIND_QUADRANT: u32 = 2u;
const KIND_RETICLE: u32 = 3u;
const KIND_LADDER: u32 = 4u;
const KIND_TAPE: u32 = 5u;

const MAX_CONTACTS: u32 = 16u;
const PI: f32 = 3.14159265;
const TAU: f32 = 6.28318530;

struct InstrumentParams {
    tint: vec4<f32>,
    warn: vec4<f32>,
    dim: vec4<f32>,
    // x kind, y value, z value2, w value3
    a: vec4<f32>,
    // x arc start (turns), y arc sweep (turns), z thickness, w tick count
    b: vec4<f32>,
    // x glow, y aspect, z contact count, w flags
    c: vec4<f32>,
};

@group(2) @binding(0) var<uniform> params: InstrumentParams;
// xy: position on the radar plane (-1..1). z: elevation (-1..1).
// w: 0 unused, 1 neutral, 2 hostile, 3 the current target.
@group(2) @binding(1) var<uniform> contacts: array<vec4<f32>, 16>;

fn aa_mask(d: f32) -> f32 {
    // One-pixel feather, derived from the local rate of change so it holds up
    // at any panel size or viewing angle.
    let w = fwidth(d) + 1e-6;
    return 1.0 - smoothstep(-w, w, d);
}

fn sd_segment(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / max(dot(ba, ba), 1e-6), 0.0, 1.0);
    return length(pa - ba * h);
}

/// Signed distance to an annulus arc: `start`/`sweep` in turns, clockwise
/// from 12 o'clock, radius `r`, half-thickness `t`.
fn sd_arc(p: vec2<f32>, start: f32, sweep: f32, r: f32, t: f32) -> f32 {
    // Angle measured clockwise from up, so gauges read like a clock face.
    var ang = atan2(p.x, p.y) / TAU;
    ang = fract(ang - start + 1.0);
    let radial = abs(length(p) - r) - t;
    if (ang <= sweep) {
        return radial;
    }
    // Past the ends: distance to the nearer cap.
    let a0 = start * TAU;
    let a1 = (start + sweep) * TAU;
    let c0 = vec2<f32>(sin(a0), cos(a0)) * r;
    let c1 = vec2<f32>(sin(a1), cos(a1)) * r;
    return min(length(p - c0), length(p - c1)) - t;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let kind = u32(params.a.x + 0.5);
    let value = params.a.y;
    let aspect = max(params.c.y, 1e-4);
    let glow = params.c.x;
    let thick = params.b.z;

    // Centred, aspect-corrected, roughly -1..1 on the short axis.
    var p = (in.uv - vec2<f32>(0.5)) * 2.0;
    p.x = p.x * aspect;
    p.y = -p.y; // UV grows downward; instruments think in +y up.

    var col = vec3<f32>(0.0);
    var alpha = 0.0;

    if (kind == KIND_ARC) {
        // Arc gauge: a track, a fill up to `value`, tick marks, and a
        // needle. The staple readout of every cockpit.
        let start = params.b.x;
        let sweep = params.b.y;
        let r = 0.72;

        let track = aa_mask(sd_arc(p, start, sweep, r, thick));
        col += params.dim.rgb * track;
        alpha = max(alpha, track * 0.55);

        let fill = aa_mask(sd_arc(p, start, sweep * clamp(value, 0.0, 1.0), r, thick));
        col += params.tint.rgb * fill * (1.0 + glow);
        alpha = max(alpha, fill);

        // Ticks around the outside.
        let ticks = params.b.w;
        if (ticks >= 1.0) {
            let n = i32(ticks);
            for (var i = 0; i <= n; i = i + 1) {
                let t = f32(i) / f32(n);
                let ang = (start + sweep * t) * TAU;
                let dir = vec2<f32>(sin(ang), cos(ang));
                let major = select(0.055, 0.10, (i % 5) == 0);
                let d = sd_segment(p, dir * (r + thick), dir * (r + thick + major)) - thick * 0.45;
                let m = aa_mask(d);
                col += params.dim.rgb * m;
                alpha = max(alpha, m * 0.8);
            }
        }

        // Needle at the current value.
        let nang = (start + sweep * clamp(value, 0.0, 1.0)) * TAU;
        let ndir = vec2<f32>(sin(nang), cos(nang));
        let nd = sd_segment(p, ndir * (r - 0.22), ndir * (r + thick * 1.6)) - thick * 0.55;
        let nm = aa_mask(nd);
        col += params.tint.rgb * nm * (1.0 + glow);
        alpha = max(alpha, nm);
    } else if (kind == KIND_RADAR) {
        // Radar plane: concentric range rings, radial spokes, a rotating
        // sweep, and contacts drawn as blips joined to the plane by a
        // vertical stalk — the stalk is what tells you a contact is above or
        // below you, and it is the single most legible idea in Elite's HUD.
        let squash = 0.42; // viewed at a shallow angle, so it reads as a disc
        var q = vec2<f32>(p.x, p.y / squash);

        for (var i = 1; i <= 3; i = i + 1) {
            let rr = f32(i) / 3.0;
            let m = aa_mask(abs(length(q) - rr) - thick * 0.5);
            col += params.dim.rgb * m;
            alpha = max(alpha, m * 0.5);
        }
        for (var i = 0; i < 4; i = i + 1) {
            let ang = f32(i) / 4.0 * TAU;
            let dir = vec2<f32>(sin(ang), cos(ang));
            let m = aa_mask(sd_segment(q, vec2<f32>(0.0), dir) - thick * 0.35);
            col += params.dim.rgb * m * 0.6;
            alpha = max(alpha, m * 0.3);
        }

        // Sweep line, one revolution every four seconds.
        let sw = fract(globals.time * 0.25) * TAU;
        let sdir = vec2<f32>(sin(sw), cos(sw));
        let sm = aa_mask(sd_segment(q, vec2<f32>(0.0), sdir) - thick * 0.5);
        col += params.tint.rgb * sm * 0.55;
        alpha = max(alpha, sm * 0.45);

        let count = u32(params.c.z + 0.5);
        for (var i = 0u; i < MAX_CONTACTS; i = i + 1u) {
            if (i >= count) { break; }
            let c = contacts[i];
            if (c.w < 0.5) { continue; }
            let base = vec2<f32>(c.x, c.y);
            let top = base + vec2<f32>(0.0, c.z * 0.55 / squash);
            // Stalk from the plane up or down to the blip.
            let stalk = aa_mask(sd_segment(q, base, top) - thick * 0.30);
            // Hostiles read in the warning colour, the current target brighter.
            var cc = params.tint.rgb;
            if (c.w > 1.5 && c.w < 2.5) { cc = params.warn.rgb; }
            let bright = select(1.0, 2.2, c.w > 2.5);
            col += cc * stalk * 0.7 * bright;
            alpha = max(alpha, stalk * 0.7);
            let blip = aa_mask(length(q - top) - thick * 1.6);
            col += cc * blip * (1.1 + glow) * bright;
            alpha = max(alpha, blip);
        }
    } else if (kind == KIND_QUADRANT) {
        // Four shield arcs around a hull ring: fore, starboard, aft, port,
        // each filling independently so damage has a direction.
        let vals = vec4<f32>(params.a.y, params.a.z, params.a.w, params.b.x);
        let r = 0.62;
        let gap = 0.022;
        for (var i = 0; i < 4; i = i + 1) {
            let start = f32(i) * 0.25 + gap - 0.125;
            let sweep = 0.25 - gap * 2.0;
            let track = aa_mask(sd_arc(p, start, sweep, r, thick));
            col += params.dim.rgb * track;
            alpha = max(alpha, track * 0.5);

            var v = vals.x;
            if (i == 1) { v = vals.y; } else if (i == 2) { v = vals.z; } else if (i == 3) { v = vals.w; }
            // Grow the fill from the middle of each arc so a weakening
            // quadrant shrinks toward its own centre rather than sliding.
            let mid = start + sweep * 0.5;
            let half = sweep * 0.5 * clamp(v, 0.0, 1.0);
            let fill = aa_mask(sd_arc(p, mid - half, half * 2.0, r, thick));
            let low = clamp(v, 0.0, 1.0) < 0.34;
            let cc = select(params.tint.rgb, params.warn.rgb, low);
            col += cc * fill * (1.0 + glow);
            alpha = max(alpha, fill);
        }
    } else if (kind == KIND_RETICLE) {
        // Four corner brackets and a centre pip. `value` closes the brackets
        // toward the centre, which is how a firing solution is signalled.
        let close = mix(0.55, 0.30, clamp(value, 0.0, 1.0));
        let arm = 0.16;
        for (var i = 0; i < 4; i = i + 1) {
            let sx = select(-1.0, 1.0, (i % 2) == 0);
            let sy = select(-1.0, 1.0, i < 2);
            let corner = vec2<f32>(sx, sy) * close;
            let h = sd_segment(p, corner, corner - vec2<f32>(sx * arm, 0.0));
            let v = sd_segment(p, corner, corner - vec2<f32>(0.0, sy * arm));
            let m = aa_mask(min(h, v) - thick * 0.5);
            col += params.tint.rgb * m * (0.95 + glow);
            alpha = max(alpha, m);
        }
        let pip = aa_mask(length(p) - thick * 1.2);
        col += params.tint.rgb * pip * (1.1 + glow);
        alpha = max(alpha, pip);
    } else if (kind == KIND_LADDER) {
        // Pitch ladder: rungs every few degrees with a gap at the centre so
        // the flight path marker stays readable. `value` is pitch in turns.
        let pitch = params.a.y;
        let spacing = 0.34;
        for (var i = -3; i <= 3; i = i + 1) {
            let y = f32(i) * spacing - fract(pitch * 8.0) * spacing;
            if (abs(y) > 1.05) { continue; }
            let inner = 0.16;
            let outer = 0.62;
            let l = sd_segment(p, vec2<f32>(-outer, y), vec2<f32>(-inner, y));
            let r = sd_segment(p, vec2<f32>(inner, y), vec2<f32>(outer, y));
            let m = aa_mask(min(l, r) - thick * 0.4);
            // Rungs fade away from the centre so the horizon reads first.
            let fade = 1.0 - clamp(abs(y), 0.0, 1.0) * 0.65;
            col += params.tint.rgb * m * fade;
            alpha = max(alpha, m * fade);
        }
    } else if (kind == KIND_TAPE) {
        // Segmented tape with a set-point pin: the Elite throttle. Segments
        // make a glance enough to read the value; the pin is where you asked
        // the engines to sit, which is not always where they are.
        let segs = max(params.b.w, 1.0);
        let n = i32(segs);
        let h = 0.62;
        for (var i = 0; i < n; i = i + 1) {
            let t0 = f32(i) / segs;
            let t1 = f32(i + 1) / segs;
            let y0 = mix(-h, h, t0) + 0.012;
            let y1 = mix(-h, h, t1) - 0.012;
            let inside = p.y > y0 && p.y < y1 && abs(p.x) < 0.30;
            if (!inside) { continue; }
            let lit = t1 <= clamp(value, 0.0, 1.0) + 1e-4;
            // Optimal-manoeuvring band, drawn even when unlit.
            let in_band = t0 >= params.a.z && t1 <= params.a.w;
            var cc = params.dim.rgb * 0.5;
            if (in_band) { cc = params.warn.rgb * 0.45; }
            if (lit) { cc = select(params.tint.rgb, params.warn.rgb, in_band) * (1.0 + glow); }
            col += cc;
            alpha = max(alpha, select(0.45, 1.0, lit));
        }
        // Set-point pin on the right edge.
        let pin_y = mix(-h, h, clamp(params.b.x, 0.0, 1.0));
        let pm = aa_mask(sd_segment(p, vec2<f32>(0.32, pin_y), vec2<f32>(0.46, pin_y)) - thick * 0.6);
        col += params.tint.rgb * pm * (1.1 + glow);
        alpha = max(alpha, pm);
    }

    if (alpha <= 0.002) {
        discard;
    }
    return vec4<f32>(col, clamp(alpha, 0.0, 1.0) * params.tint.a);
}
