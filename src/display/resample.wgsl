// A picture resized by any factor without moire (the CTM's scanlines and phosphor mask beaten into
// stripes): the way of the Benediction website's thumbnails (bnd.website, tools/thumbnails.py) -
// a Gaussian blur proportional to the reduction factor, then a Lanczos (a = 3) resampling.
// Both are separable: one pass along x into a texture of the destination's width, one along y
// into the viewport. Each output pixel folds the two kernels into one weight per source texel.

struct Params {
    // the size of this pass's source, in texels
    src: vec2<f32>,
    // the size of what this pass draws, in pixels
    dst: vec2<f32>,
    // 0: along x, 1: along y
    axis: f32,
    // the blur before the Lanczos, in source texels (`display::resize::blur_sigma`)
    sigma: f32,
    // to 32 bytes: WebGL wants uniforms of a multiple of 16
    pad: vec2<f32>,
}

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var<uniform> params: Params;

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

const PI: f32 = 3.14159265;
// the source texels one output pixel may read
const TAPS: i32 = 48;

fn lanczos3(x: f32) -> f32 {
    let a = abs(x);
    if (a < 1e-4) {
        return 1.0;
    }
    if (a >= 3.0) {
        return 0.0;
    }
    return 3.0 * sin(PI * x) * sin(PI * x / 3.0) / (PI * PI * x * x);
}

@fragment
fn fs(v: Vertex) -> @location(0) vec4<f32> {
    let along_x = params.axis < 0.5;
    let n_src = select(params.src.y, params.src.x, along_x);
    let n_dst = select(params.dst.y, params.dst.x, along_x);
    // where the output pixel's centre falls in the source, in texels along the axis
    let p = select(v.uv.y, v.uv.x, along_x) * n_src;
    // the texel across (the same size both sides of the pass)
    let across_n = select(params.src.x, params.src.y, along_x);
    let across = clamp(i32(floor(select(v.uv.x, v.uv.y, along_x) * across_n)), 0, i32(across_n) - 1);

    let zoom = n_dst / n_src;
    // the Lanczos at the output's scale when reducing, the source's when enlarging
    let s = clamp(zoom, 0.25, 1.0);
    let sigma = params.sigma;
    let lanczos_r = 3.0 / s;
    let gauss_r = i32(ceil(3.0 * sigma));
    let k0 = i32(floor(p - lanczos_r));
    let k1 = i32(ceil(p + lanczos_r));
    let j0 = k0 - gauss_r;

    // the weight of each source texel: the Lanczos of the blurred texels around it
    var w: array<f32, TAPS>;
    for (var i = 0; i < TAPS; i++) {
        w[i] = 0.0;
    }
    var g_sum = 0.0;
    for (var d = -gauss_r; d <= gauss_r; d++) {
        g_sum += exp(-f32(d * d) / max(2.0 * sigma * sigma, 1e-6));
    }
    for (var k = k0; k <= k1; k++) {
        let lw = lanczos3((p - (f32(k) + 0.5)) * s);
        if (lw == 0.0) {
            continue;
        }
        for (var d = -gauss_r; d <= gauss_r; d++) {
            let g = exp(-f32(d * d) / max(2.0 * sigma * sigma, 1e-6)) / g_sum;
            let i = k + d - j0;
            if (i >= 0 && i < TAPS) {
                w[i] += lw * g;
            }
        }
    }
    var colour = vec3<f32>(0.0);
    var total = 0.0;
    let last = i32(n_src) - 1;
    for (var i = 0; i < TAPS; i++) {
        if (w[i] == 0.0) {
            continue;
        }
        let j = clamp(j0 + i, 0, last);
        let texel = select(vec2<i32>(across, j), vec2<i32>(j, across), along_x);
        colour += w[i] * textureLoad(source, texel, 0).rgb;
        total += w[i];
    }
    return vec4<f32>(clamp(colour / max(total, 1e-6), vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}
