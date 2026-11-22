//! The CRT filter that reads the picture back (a [`DisplayFilter`]): recordings, screenshots, the
//! examples. It runs the CubeCL kernels of `kernel.rs` on a client of its own, on the GPU (wgpu) or
//! on CubeCL's CPU backend (LLVM, where a build of it exists: Linux x86_64 / aarch64, macOS aarch64,
//! Windows x86_64). The window and the web page draw the kernels' picture directly instead
//! (`CrtPass`).

use super::crt::CrtParams;
use super::crt::Warp;
use super::kernel::{frame_bytes, Pipeline};
use super::DisplayFilter;
use cubecl_runtime::runtime::Runtime;
use image::RgbImage;
use std::sync::OnceLock;

/// Where the kernels run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    /// A GPU adapter, through wgpu.
    Gpu,
    /// CubeCL's CPU backend: no adapter, the same picture on any machine (to rounding).
    Cpu,
}

type Client = cubecl::client::Client;

fn gpu_client() -> Result<Client, String> {
    static GPU: OnceLock<Result<Client, String>> = OnceLock::new();
    GPU.get_or_init(|| {
        let device = cubecl::wgpu::WgpuDevice::default();
        cubecl::wgpu::try_init_setup::<cubecl::wgpu::AutoGraphicsApi>(&device, Default::default())
            .map_err(|e| format!("no GPU adapter: {e:?}"))?;
        Ok(<cubecl::wgpu::WgpuRuntime as Runtime>::client(&device))
    })
    .clone()
}

#[cfg(crt_cpu)]
fn cpu_client() -> Result<Client, String> {
    static CPU: OnceLock<Client> = OnceLock::new();
    Ok(CPU
        .get_or_init(|| <cubecl::cpu::CpuRuntime as Runtime>::client(&Default::default()))
        .clone())
}

#[cfg(not(crt_cpu))]
fn cpu_client() -> Result<Client, String> {
    Err("this build has no software rendering of the CRT on this platform".into())
}

/// The CRT model of [`CrtParams`], through the glass `warp`, computed by the kernels of
/// `kernel.rs`.
pub struct KernelCrt {
    params: CrtParams,
    backend: Backend,
    client: Client,
    pipeline: Pipeline,
}

impl KernelCrt {
    /// Fails (with a message) where `backend` is not available.
    pub fn new(params: CrtParams, warp: Warp, backend: Backend) -> Result<Self, String> {
        params.validate()?;
        let client = match backend {
            Backend::Gpu => gpu_client()?,
            Backend::Cpu => cpu_client()?,
        };
        let mut pipeline = Pipeline::new(&client, &params, warp);
        // a cube runs on one thread of the CPU backend: small ones spread the picture over them
        pipeline.cube_dim = if backend == Backend::Cpu { 32 } else { 256 };
        Ok(Self {
            params,
            backend,
            client,
            pipeline,
        })
    }

    pub fn backend(&self) -> Backend {
        self.backend
    }
}

impl DisplayFilter for KernelCrt {
    fn output_size(&self) -> (usize, usize) {
        self.params.output_size()
    }

    fn render(&mut self, src: &RgbImage, dst: &mut [u32]) {
        let (w, h) = self.params.output_size();
        assert_eq!(dst.len(), w * h);
        let (bytes, sw, sh) = frame_bytes(src);
        let frame = self.client.create_from_slice(&bytes);
        self.pipeline.run(&self.client, &frame, bytes.len() / 4, sw, sh);
        let picture = self
            .client
            .read_one(self.pipeline.out().clone())
            .expect("cannot read the CRT picture");
        for (pixel, word) in dst.iter_mut().zip(picture.chunks_exact(4)) {
            *pixel = u32::from_le_bytes([word[0], word[1], word[2], word[3]]);
        }
    }
}
