// Draws a texture over the viewport the frontend set (the picture's aspect
// ratio, centred in the window), seen through the tube's curved glass: the
// `Warp` of `crt.rs`, the same formula as its CPU version.

struct Warp {
    // horizontal and vertical bulge, 0 when flat
    k: vec2<f32>,
    // radius of the glass's corners, in half-sizes of the picture
    corner: f32,
    // 1 to cut the picture at the glass's edge
    edged: f32,
}

@group(0) @binding(0) var picture: texture_2d<f32>;
@group(0) @binding(1) var picture_sampler: sampler;
@group(0) @binding(2) var<uniform> warp: Warp;

struct Vertex {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs(@builtin(vertex_index) i: u32) -> Vertex {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    var v: Vertex;
    v.position = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    v.uv = vec2<f32>(x, y);
    return v;
}

@fragment
fn fs(v: Vertex) -> @location(0) vec4<f32> {
    let p = v.uv * 2.0 - 1.0;
    let w = p * (1.0 + warp.k * p.yx * p.yx);
    // distance to the rounded rectangle |w| <= 1 (negative inside), its edge
    // smoothed over about a pixel
    let q = abs(w) - (1.0 - warp.corner);
    let distance = length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - warp.corner;
    let pixel = min(fwidth(p.x), fwidth(p.y));
    let alpha = mix(1.0, clamp(0.5 - distance / pixel, 0.0, 1.0), warp.edged);
    let colour = textureSample(picture, picture_sampler, (w + 1.0) / 2.0).rgb;
    return vec4<f32>(colour * alpha, 1.0);
}
