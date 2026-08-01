// World-space UI panel: composites a rendered UI texture with one of three
// skin treatments. One pipeline, three branches, so switching skins at
// runtime costs a uniform write rather than a shader swap.
//
// The UI texture arrives premultiplied-ish from a 2D camera that cleared to
// transparent black, so its alpha already describes where content is.

#import bevy_pbr::forward_io::VertexOutput
#import bevy_pbr::mesh_view_bindings::globals

const SKIN_HOLOGRAPHIC: u32 = 0u;
const SKIN_INDUSTRIAL: u32 = 1u;
const SKIN_MINIMAL: u32 = 2u;
const SKIN_TEXTURED: u32 = 3u;

struct PanelParams {
    accent: vec4<f32>,
    text_tint: vec4<f32>,
    backdrop: vec4<f32>,
    // x emissive, y backdrop_opacity, z scanline_strength, w edge_glow
    a: vec4<f32>,
    // x flicker, y bezel, z corner_radius, w aspect (width / height)
    b: vec4<f32>,
    // x skin mode, y selected (0/1), z opacity, w unused
    c: vec4<f32>,
    // x,y source border fraction; z,w panel border fraction (nine-slice)
    d: vec4<f32>,
};

@group(2) @binding(0) var<uniform> params: PanelParams;
@group(2) @binding(1) var content_tex: texture_2d<f32>;
@group(2) @binding(2) var content_sampler: sampler;
@group(2) @binding(3) var frame_tex: texture_2d<f32>;
@group(2) @binding(4) var frame_sampler: sampler;
@group(2) @binding(5) var backdrop_tex: texture_2d<f32>;
@group(2) @binding(6) var backdrop_sampler: sampler;

/// Map a panel coordinate to a source coordinate for one nine-slice axis.
///
/// Corners sample the source corners at fixed size; the middle stretches.
/// Without this a wide panel smears its corner bevels into ovals.
fn nine_slice_axis(t: f32, src_border: f32, panel_border: f32) -> f32 {
    if (panel_border <= 0.0 || src_border <= 0.0) {
        return t;
    }
    if (t < panel_border) {
        return t / panel_border * src_border;
    }
    if (t > 1.0 - panel_border) {
        return 1.0 - (1.0 - t) / panel_border * src_border;
    }
    let span = max(1.0 - 2.0 * panel_border, 1e-5);
    return src_border + (t - panel_border) / span * (1.0 - 2.0 * src_border);
}

/// Signed distance to a rounded box centred at the origin, half-size `b`,
/// corner radius `r`. Negative inside.
fn sd_round_box(p: vec2<f32>, b: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - b + vec2<f32>(r);
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

fn luminance(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let mode = u32(params.c.x + 0.5);
    let emissive = params.a.x;
    let backdrop_opacity = params.a.y;
    let scanline_strength = params.a.z;
    let edge_glow = params.a.w;
    let flicker_amt = params.b.x;
    let bezel = params.b.y;
    let corner_radius = params.b.z;
    let aspect = max(params.b.w, 0.0001);
    let selected = params.c.y;
    let panel_opacity = params.c.z;

    var uv = in.uv;

    // Industrial screens sit behind glass: bow the sampled coordinates so the
    // content looks like it is painted on the inside of a curved tube.
    if (mode == SKIN_INDUSTRIAL) {
        let c = (uv - vec2<f32>(0.5)) * 2.0;
        let r2 = dot(c, c);
        uv = vec2<f32>(0.5) + c * (1.0 + 0.035 * r2) * 0.5;
    }

    // Outside the (possibly bowed) content rectangle there is nothing to
    // sample; let the bezel or the panel edge handle it.
    let outside = any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0));
    var content = vec4<f32>(0.0);
    if (!outside) {
        content = textureSample(content_tex, content_sampler, uv);
    }

    // Rounded-rectangle mask in aspect-corrected panel space.
    let p = (in.uv - vec2<f32>(0.5)) * vec2<f32>(aspect, 1.0);
    let half = vec2<f32>(0.5 * aspect, 0.5);
    let radius = corner_radius * min(aspect, 1.0);
    let d = sd_round_box(p, half, radius);
    // Anti-alias the silhouette over roughly one pixel of panel space.
    let aa = fwidth(d) + 1e-5;
    let inside_mask = 1.0 - smoothstep(-aa, aa, d);
    if (inside_mask <= 0.001) {
        discard;
    }

    // Slow brightness wobble; deterministic in time, no per-frame randomness.
    var flicker = 1.0;
    if (flicker_amt > 0.0) {
        let t = globals.time;
        flicker = 1.0 + flicker_amt * (sin(t * 31.0) * 0.6 + sin(t * 7.3) * 0.4);
    }

    // Horizontal scanlines, in content pixels rather than screen pixels so
    // they stay put as the panel moves.
    var scan = 1.0;
    if (scanline_strength > 0.0) {
        let lines = sin(in.uv.y * 900.0 - globals.time * 2.0);
        scan = 1.0 - scanline_strength * (0.5 + 0.5 * lines);
    }

    // Fresnel-ish rim: brightest where the surface turns away from us.
    let view_dir = normalize(in.world_position.xyz - vec3<f32>(0.0));
    let facing = abs(dot(normalize(in.world_normal), normalize(view_dir)));
    let rim = pow(1.0 - clamp(facing, 0.0, 1.0), 2.5);

    // Border: a bright line just inside the silhouette.
    let border_w = max(radius * 0.35, 0.004);
    let border = 1.0 - smoothstep(0.0, border_w, abs(d + border_w));

    var rgb: vec3<f32>;
    var alpha: f32;

    if (mode == SKIN_HOLOGRAPHIC) {
        // Projected light: content is additive, the body is a faint wash, and
        // dark areas of the content are genuinely see-through.
        let body = params.backdrop.rgb * backdrop_opacity;
        rgb = (body + content.rgb * emissive) * scan * flicker;
        rgb += params.accent.rgb * rim * edge_glow * 0.9;
        rgb += params.accent.rgb * border * 1.6;
        let content_a = max(content.a, luminance(content.rgb));
        alpha = clamp(backdrop_opacity + content_a + border * 0.8 + rim * 0.25, 0.0, 1.0);
    } else if (mode == SKIN_INDUSTRIAL) {
        // A screen inset into a metal frame. The bezel is shaded by hand:
        // lighter at the top, darker at the bottom, so it reads as lit.
        let frame = smoothstep(-bezel, -bezel * 0.35, d);
        let metal_base = vec3<f32>(0.20, 0.20, 0.22);
        let lit = mix(1.35, 0.55, clamp(in.uv.y, 0.0, 1.0));
        let metal = metal_base * lit;

        // Vignette the screen so the corners fall off like a real CRT.
        let vig = 1.0 - 0.45 * dot(p / half, p / half);
        var screen_rgb = params.backdrop.rgb + content.rgb * emissive;
        screen_rgb = screen_rgb * scan * flicker * clamp(vig, 0.0, 1.0);
        if (outside) {
            screen_rgb = params.backdrop.rgb * 0.4;
        }

        rgb = mix(screen_rgb, metal, frame);
        rgb += params.accent.rgb * border * 0.35;
        alpha = 1.0;
    } else if (mode == SKIN_TEXTURED) {
        // Art-driven: a nine-sliced frame over a nine-sliced body, both
        // tinted by the palette so one frame serves many moods.
        let src = vec2<f32>(
            nine_slice_axis(in.uv.x, params.d.x, params.d.z),
            nine_slice_axis(in.uv.y, params.d.y, params.d.w),
        );
        let body = textureSample(backdrop_tex, backdrop_sampler, src);
        let frame = textureSample(frame_tex, frame_sampler, src);

        // Body first, then the UI content, then the frame on top so the
        // bevel always reads as being in front of the text.
        var acc = params.backdrop.rgb * body.rgb;
        var acc_a = body.a * backdrop_opacity;
        acc = mix(acc, content.rgb * emissive, content.a);
        acc_a = clamp(acc_a + content.a, 0.0, 1.0);

        let frame_rgb = frame.rgb * params.accent.rgb * (1.0 + selected * 0.6);
        acc = mix(acc, frame_rgb, frame.a);
        acc_a = clamp(acc_a + frame.a, 0.0, 1.0);

        acc = acc * flicker;
        acc += params.accent.rgb * rim * edge_glow * 0.35;

        rgb = acc;
        alpha = acc_a;
    } else {
        // Minimal: dark glass, one thin accent rule, crisp content.
        let body = params.backdrop.rgb;
        rgb = body + content.rgb * emissive;
        rgb += params.accent.rgb * border * 0.85;
        rgb += params.accent.rgb * rim * edge_glow * 0.25;
        let content_a = max(content.a, luminance(content.rgb));
        alpha = clamp(backdrop_opacity + content_a * 0.6 + border * 0.9, 0.0, 1.0);
    }

    // Selection is a skin-independent affordance: whatever the look, the
    // panel you are acting on gets brighter and its edge lifts.
    if (selected > 0.5) {
        rgb += params.accent.rgb * (0.18 + border * 1.2);
        alpha = clamp(alpha + 0.10, 0.0, 1.0);
    }

    return vec4<f32>(rgb, alpha * inside_mask * panel_opacity);
}
