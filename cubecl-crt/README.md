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
* Not done: the browser (WebGPU), the quad pass that draws the buffer, `Picture` integration.
