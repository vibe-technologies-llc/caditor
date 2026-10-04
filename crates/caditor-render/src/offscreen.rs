use std::sync::Arc;

use crate::{
    GraphicsSettings, RenderError,
    gpu::{self, DeviceLoss},
    image::{
        IMAGE_FORMAT, Image, ImageError, ImagePoll, ImageRequest, PendingImage, TILE_SIDE,
        check_size,
    },
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
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let opened = pollster::block_on(gpu::open_device(&instance, None))?;
        let loss = DeviceLoss::watch(&opened.device, Arc::new(|| {}));
        let offered =
            gpu::offered_msaa(&opened.adapter, &opened.device, IMAGE_FORMAT, DEPTH_FORMAT);
        let mut viewport = ViewportRenderer::new(
            &opened.device,
            IMAGE_FORMAT,
            graphics.msaa.closest(&offered).samples(),
        );
        viewport.set_shading(graphics.shading);
        Ok(Self {
            device: opened.device,
            queue: opened.queue,
            loss,
            viewport,
        })
    }

    pub fn render(&mut self, request: &ImageRequest<'_>) -> Result<Image, ImageError> {
        check_size(request.size)?;
        if self.loss.is_lost() {
            return Err(ImageError::DeviceLost);
        }
        let tile_side = TILE_SIDE.min(self.largest_side()).max(1);
        let readback = crate::encode_checked(&self.device, || {
            self.viewport
                .encode_image(&self.device, &self.queue, request, tile_side)
        })?;
        let pending = PendingImage::map(readback, 0);
        if let Err(error) = self.device.poll(wgpu::PollType::wait_indefinitely()) {
            log::warn!("could not wait for the graphics device to draw an image: {error}");
            return Err(ImageError::DeviceLost);
        }
        if self.loss.is_lost() {
            return Err(ImageError::DeviceLost);
        }
        if !pending.is_mapped() {
            return Err(ImageError::Readback);
        }
        match pending.finish() {
            ImagePoll::Ready(readback) => readback.into_image(),
            ImagePoll::Failed(error) => Err(error),
            ImagePoll::Idle | ImagePoll::Pending => Err(ImageError::Readback),
        }
    }

    fn largest_side(&self) -> u32 {
        self.device.limits().max_texture_dimension_2d
    }
}
