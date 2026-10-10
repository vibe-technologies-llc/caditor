use std::sync::Arc;

use crate::{
    GraphicsSettings, RenderError,
    gpu::{self, DeviceLoss},
    image::{self, IMAGE_FORMAT, ImageBands, ImageError, ImageGpu, ImageRequest, TILE_SIDE},
    viewport::{DEPTH_FORMAT, ViewportRenderer},
};

pub struct OffscreenRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    loss: DeviceLoss,
    viewport: ViewportRenderer,
}

impl OffscreenRenderer {
    pub fn new(graphics: GraphicsSettings) -> Result<Self, RenderError> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let opened = pollster::block_on(gpu::open_device(&instance, None, graphics.adapter))?;
        let loss = DeviceLoss::watch(&opened.device, Arc::new(|| {}));
        let offered =
            gpu::offered_msaa(&opened.adapter, &opened.device, IMAGE_FORMAT, DEPTH_FORMAT);
        let mut viewport = ViewportRenderer::new(
            &opened.device,
            IMAGE_FORMAT,
            graphics.msaa.closest(&offered).samples(),
        );
        viewport.set_shading(graphics.shading);
        viewport.set_linear_resolve(gpu::resolves_linearly(&opened.adapter));
        Ok(Self {
            device: opened.device,
            queue: opened.queue,
            loss,
            viewport,
        })
    }

    pub fn set_background(&mut self, background: wgpu::Color) {
        self.viewport.set_background(background);
    }

    pub fn render(&mut self, request: &ImageRequest<'_>) -> Result<ImageBands, ImageError> {
        let tile_side = TILE_SIDE.min(self.device.limits().max_texture_dimension_2d);
        let viewport = &self.viewport;
        let device = &self.device;
        image::drawn_inline(
            ImageGpu {
                device: self.device.clone(),
                queue: self.queue.clone(),
                loss: self.loss.clone(),
            },
            || viewport.image_sibling(device),
            request,
            tile_side,
        )
    }
}
