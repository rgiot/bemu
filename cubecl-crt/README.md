# CubeCL spike: one source for the CRT model

`src/main.rs` is `src/display/crt.wgsl` (flat glass, colour and mono tubes, halation, veil, glow,
vignette) written once as a CubeCL `#[cube]` kernel, run through CubeCL's wgpu backend and compared
with bemu's CPU renderer (`CpuCrt`).

`cargo run --release` (needs a Vulkan/Metal/WebGPU adapter; `mesa-vulkan-drivers` gives the software
one, lavapipe).

Result on lavapipe (software Vulkan, so the times say nothing about a real GPU): CTM644 and GT65 at
1440x1088, worst difference 1 level out of 255 on 4.7 M channel values, none above 3.
What it showed: `exp`, `sqrt`, `floor`, `clamp`, `min`/`max`, loops, `if` chains, `i32`/`u32`/`f32`
casts and slices (`&[f32]`, `&[u32]`) all work in `#[cube]` 0.11.4. CubeCL's CPU backend needs LLVM
(downloaded by its build script: not tried).
Not done: curved glass, a direct draw of the output buffer (no readback), the browser.
