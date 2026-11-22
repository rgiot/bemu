# CubeCL spike: one source for the CRT model

`src/main.rs` is `src/display/crt.wgsl` (flat glass, colour and mono tubes, halation, veil, glow,
vignette) written once as a CubeCL `#[cube]` kernel, run through CubeCL's wgpu backend and compared
with bemu's CPU renderer (`CpuCrt`).

`cargo run --release` (needs a Vulkan/Metal/WebGPU adapter; `mesa-vulkan-drivers` gives the software
one, lavapipe).

Results on lavapipe (software Vulkan, so the times say nothing about a real GPU), 1440x1088, 4.7 M
channel values each:

| tube, glass | worst difference | values off by more than 3 |
|---|---|---|
| CTM644, flat | 1 | 0 |
| CTM644, curved | 5 | 24 (0.0005 %) |
| GT65 (mono), curved | 1 | 0 |

* `exp`, `sqrt`, `floor`, `clamp`, `min`/`max`, loops, `if` chains, integer/float casts and slices
  (`&[f32]`, `&[u32]`) all work in `#[cube]` 0.11.4.
* **Drawing without readback**: `shared_device_check` gives CubeCL our own wgpu device
  (`init_device(WgpuSetup{..})`) and takes a kernel's output back as a `wgpu::Buffer` of that device
  (`client.get_resource::<WgpuServer<AutoCompiler>>(handle)`), usage `STORAGE | COPY_SRC | COPY_DST`
  at a 4096-byte offset: a render pass can bind it read-only. Copied with our own queue, the data is
  the kernel's.
* **Version constraint**: cubecl 0.11.4 uses **wgpu 30**; bemu is on wgpu 27 and egui-wgpu 0.33.
  egui-wgpu 0.37 is on wgpu 30: adopting CubeCL means moving bemu to wgpu 30 / egui 0.37.
* CubeCL's CPU backend needs an LLVM bundle downloaded by its build script (not tried).
* **The whole path of a frame without readback** (`quad_pass_check`): kernel -> buffer of the shared
  device -> a fullscreen pass that binds it as read-only storage (`BufferBinding` with CubeCL's offset)
  and draws it into a texture. Read back, it is the kernel's picture: worst difference 5, 24 values
  off by more than 3, against the CPU renderer (curved CTM644).
* **The browser**: the same kernel (`src/kernel.rs`, included unchanged) built for wasm32, initialised
  with `init_setup_async::<WebGpu>` and read with `read_async`, runs in headless Chromium (WebGPU on
  SwiftShader, Playwright): CTM644 curved, 1440x1088, same first pixels as the native run and a sum of
  the picture within 0.01 % of it (rounding of `exp`/`sqrt` between the two adapters). The `.wasm` is
  29 MB (13 MB gzipped) before any `wasm-opt`.
* Not done: the integration into `Picture` (the quad pass replaces `CrtPass`).
