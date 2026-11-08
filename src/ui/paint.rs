//! Draws the interface with wgpu, over what is already in the target (the picture).

/// egui's renderer, on the device of the window or the page.
pub struct Painter {
    renderer: egui_wgpu::Renderer,
}

impl Painter {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        Self {
            renderer: egui_wgpu::Renderer::new(
                device,
                format,
                egui_wgpu::RendererOptions::default(),
            ),
        }
    }

    /// Draw `output` (from [`Gui::run`](super::Gui::run)) on `view`, `size` pixels with
    /// `pixels_per_point`, keeping what is there.
    #[allow(clippy::too_many_arguments)]
    pub fn paint(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        size: [u32; 2],
        ctx: &egui::Context,
        output: egui::FullOutput,
    ) {
        let pixels_per_point = output.pixels_per_point;
        let jobs = ctx.tessellate(output.shapes, pixels_per_point);
        for (id, delta) in &output.textures_delta.set {
            self.renderer.update_texture(device, queue, *id, delta);
        }
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: size,
            pixels_per_point,
        };
        let commands = self
            .renderer
            .update_buffers(device, queue, encoder, &jobs, &screen);
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("egui"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                })
                .forget_lifetime();
            self.renderer.render(&mut pass, &jobs, &screen);
        }
        if !commands.is_empty() {
            queue.submit(commands);
        }
        for id in &output.textures_delta.free {
            self.renderer.free_texture(id);
        }
    }
}
