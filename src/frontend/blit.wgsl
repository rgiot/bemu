// Draws a texture over the viewport the frontend set (the picture's aspect
// ratio, centred in the window).

@group(0) @binding(0) var picture: texture_2d<f32>;
@group(0) @binding(1) var picture_sampler: sampler;

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
    return vec4<f32>(textureSample(picture, picture_sampler, v.uv).rgb, 1.0);
}
