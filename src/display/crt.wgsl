// The CRT model of `crt.rs`, evaluated per output pixel on the GPU.
//
// The approach (per-gun sub-pixel beam sampling, 480 triads across the
// picture, spill of bright slots, staggered slot mask, halation) is inspired
// by the rendering source code of the ACE-DL emulator (CTM_Render.c); this is
// an independent reimplementation, line for line the one of `cpu.rs`.

struct Params {
    out_w: u32,
    out_h: u32,
    slot_px: u32,
    rows_per_line: u32,
    pitch: f32,
    fill_x: f32,
    fill_y: f32,
    corner: f32,
    stagger: f32,
    sigma_h: f32,
    sigma_v: f32,
    spill: f32,
    halation: f32,
    scale: f32,
    x_ratio: f32,
    // the glass (`Warp` of crt.rs): bulge, corners' radius, 1 if curved
    warp_kx: f32,
    warp_ky: f32,
    warp_corner: f32,
    edged: f32,
    pad: f32,
}

@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var src: texture_2d<f32>;

const SRC_W: i32 = 768;
const SRC_H: i32 = 272;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    // one triangle covering the screen
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}

fn gauss(d: f32, sigma: f32) -> f32 {
    return exp(-d * d / (2.0 * sigma * sigma));
}

// Linear light of gun `c` of the monitor pixel (i, j); black outside.
fn gun(i: i32, j: i32, c: u32) -> f32 {
    if (i < 0 || i >= SRC_W || j < 0 || j >= SRC_H) {
        return 0.0;
    }
    let v = textureLoad(src, vec2<i32>(i, j), 0);
    var s = v.r;
    if (c == 1u) { s = v.g; } else if (c == 2u) { s = v.b; }
    return s * s;
}

fn rounded_box_distance(px: f32, py: f32, hx: f32, hy: f32, r: f32) -> f32 {
    let qx = abs(px) - (hx - r);
    let qy = abs(py) - (hy - r);
    let outside = length(vec2<f32>(max(qx, 0.0), max(qy, 0.0)));
    return outside + min(max(qx, qy), 0.0) - r;
}

fn slot_distance(x: i32, y: i32) -> f32 {
    let sw = i32(p.slot_px);
    let slot_w = f32(sw);
    let slot = x / sw;
    let triad = slot / 3;
    var shift = 0.0;
    if (triad % 2 == 1) { shift = p.stagger * p.pitch; }
    let px = (f32(x) + 0.5) - f32(slot) * slot_w - slot_w / 2.0;
    let t = f32(y) + 0.5 - shift;
    let py = (t - p.pitch * floor(t / p.pitch)) - p.pitch / 2.0;
    let hx = p.fill_x * slot_w / 2.0;
    let hy = p.fill_y * p.pitch / 2.0;
    let r = max(p.corner * min(hx, hy), 0.0);
    return rounded_box_distance(px, py, hx, hy, r);
}

// The point of the flat picture (in [0, 1]^2) the glass shows at `uv`: the `Warp` of crt.rs.
fn warped(uv: vec2<f32>) -> vec2<f32> {
    let q = uv * 2.0 - 1.0;
    let w = q * (1.0 + vec2<f32>(p.warp_kx, p.warp_ky) * q.yx * q.yx);
    return (w + 1.0) / 2.0;
}

// Light of the slot at output column `x`, row `y`, in its own gun's channel. Through curved
// glass the beam's raster bends, not the mask: the slot reads the beam where its centre's warped
// place is (no resampling of the mask, so no moire).
fn light(x: i32, y: i32) -> vec3<f32> {
    if (x < 0 || x >= i32(p.out_w)) {
        return vec3<f32>(0.0);
    }
    let sw = i32(p.slot_px);
    let slot = x / sw;
    let c = u32(slot % 3);
    let size = vec2<f32>(f32(p.out_w), f32(p.out_h));
    let beam = warped(vec2<f32>((f32(slot) + 0.5) * f32(sw), f32(y) + 0.5) / size) * size;
    let u = beam.x / p.x_ratio;
    let first = i32(floor(u - 0.5)) - 1;
    var w: array<f32, 4>;
    var sum = 0.0;
    for (var t = 0; t < 4; t++) {
        w[t] = gauss(u - (f32(first + t) + 0.5), p.sigma_h);
        sum += w[t];
    }
    let f = beam.y / f32(p.rows_per_line) - 0.5;
    let line = i32(floor(f));
    let a = gauss(f - f32(line), p.sigma_v);
    let b = gauss(f32(line) + 1.0 - f, p.sigma_v);
    var la = 0.0;
    var lb = 0.0;
    for (var t = 0; t < 4; t++) {
        la += w[t] / sum * gun(first + t, line, c);
        lb += w[t] / sum * gun(first + t, line + 1, c);
    }
    let level = a * la + b * lb;
    let coverage = clamp(0.5 - (slot_distance(x, y) - p.spill * level), 0.0, 1.0);
    var o = vec3<f32>(0.0);
    o[c] = level * coverage * p.scale;
    return o;
}

@fragment
fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let x = i32(pos.x);
    let y = i32(pos.y);
    let sw = i32(p.slot_px);
    let rgb = (1.0 - 2.0 * p.halation) * light(x, y) + p.halation * (light(x - sw, y) + light(x + sw, y));
    // the glass's rounded edge, smoothed over about a pixel (as the blit of a flat picture)
    let q = warped((vec2<f32>(f32(x), f32(y)) + 0.5) / vec2<f32>(f32(p.out_w), f32(p.out_h))) * 2.0 - 1.0;
    let e = abs(q) - (1.0 - p.warp_corner);
    let distance = length(max(e, vec2<f32>(0.0))) + min(max(e.x, e.y), 0.0) - p.warp_corner;
    let pixel = 2.0 / f32(max(p.out_w, p.out_h));
    let alpha = mix(1.0, clamp(0.5 - distance / pixel, 0.0, 1.0), p.edged);
    return vec4<f32>(sqrt(clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0))) * alpha, 1.0);
}
