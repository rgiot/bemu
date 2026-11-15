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
    // 1 to keep the texels sharp at any zoom (sharp bilinear), 0 for a plain linear sample
    sharp: f32,
    // to 32 bytes: WebGL wants uniforms of a multiple of 16
    pad0: f32,
    pad1: vec2<f32>,
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
    let uv = (w + 1.0) / 2.0;
    // the zoom: screen pixels per texel, along each axis
    let size = vec2<f32>(textureDimensions(picture));
    let texel = uv * size;
    let zoom = 1.0 / max(fwidth(texel), vec2<f32>(1e-6));
    var colour: vec3<f32>;
    if (warp.sharp < 0.5) {
        colour = textureSampleLevel(picture, picture_sampler, uv, 0.0).rgb;
    } else if (zoom.x >= 1.0 && zoom.y >= 1.0) {
        // sharp bilinear: each texel a solid block, only the screen pixel on its edge mixes the
        // two (no uneven widths, no moiré, no blur; exactly nearest at a whole zoom)
        let base = floor(texel - 0.5) + 0.5;
        let f = clamp((texel - base - 0.5) * zoom + 0.5, vec2<f32>(0.0), vec2<f32>(1.0));
        colour = textureSampleLevel(picture, picture_sampler, (base + f) / size, 0.0).rgb;
    } else {
        // smaller than the picture: the texels under the pixel averaged (4 linear taps)
        let d = 0.25 / min(zoom, vec2<f32>(1.0)) / size;
        colour = (textureSampleLevel(picture, picture_sampler, uv + vec2<f32>(-d.x, -d.y), 0.0).rgb
            + textureSampleLevel(picture, picture_sampler, uv + vec2<f32>(d.x, -d.y), 0.0).rgb
            + textureSampleLevel(picture, picture_sampler, uv + vec2<f32>(-d.x, d.y), 0.0).rgb
            + textureSampleLevel(picture, picture_sampler, uv + vec2<f32>(d.x, d.y), 0.0).rgb) / 4.0;
    }
    return vec4<f32>(colour * alpha, 1.0);
}
