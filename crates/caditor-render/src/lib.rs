mod camera;
mod gpu;
mod mesh;
#[cfg(test)]
mod offscreen_tests;
mod picking;
mod scene;
mod viewport;

use std::{fmt::Debug, sync::Arc};

use wgpu::rwh::{HasDisplayHandle, HasWindowHandle};

pub use crate::{
    camera::{Camera, View, Viewpoint},
    gpu::Wake,
    mesh::{FaceStyle, MeshFace, MeshInstance, MeshPoint, ShadedMesh},
    picking::PickPoll,
    scene::{
        Color, Fill, Grid, Layer, Line, Marker, PickHit, PickId, PickResult, Scene, ViewportRect,
    },
    viewport::ViewportFrame,
};
use crate::{
    gpu::DeviceLoss,
    viewport::{DEPTH_FORMAT, SurfaceTarget, ViewportRenderer},
};

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("could not create a drawing surface for the window: {0}")]
    CreateSurface(#[from] wgpu::CreateSurfaceError),
    #[error("no graphics adapter can draw to this window")]
    NoAdapter,
    #[error("no graphics adapter would open a device: {0}")]
    RequestDevice(#[from] wgpu::RequestDeviceError),
    #[error("the graphics adapter cannot present to this window")]
    UnsupportedSurface,
    #[error("the graphics device refused the window's drawing surface")]
    ConfigureSurface,
    #[error("the window's drawing surface raised a validation error")]
    SurfaceValidation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SurfaceSize {
    pub width: u32,
    pub height: u32,
}

pub enum FrameStart {
    Ready(Box<Frame>),
    Hidden,
    Skipped,
}

pub struct Frame {
    surface_texture: wgpu::SurfaceTexture,
    generation: u64,
    pub view: wgpu::TextureView,
    pub encoder: wgpu::CommandEncoder,
}

pub trait WindowTarget: HasWindowHandle + HasDisplayHandle + Debug + Send + Sync + 'static {}

impl<T> WindowTarget for T where
    T: HasWindowHandle + HasDisplayHandle + Debug + Send + Sync + 'static
{
}

struct Gpu {
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    loss: DeviceLoss,
}

impl Gpu {
    async fn open(
        instance: &wgpu::Instance,
        surface: &wgpu::Surface<'static>,
        size: SurfaceSize,
        wake: &Wake,
    ) -> Result<Self, RenderError> {
        let opened = gpu::open_device(instance, Some(surface)).await?;
        let loss = DeviceLoss::watch(&opened.device, Arc::clone(wake));
        let config = configure(&opened.adapter, &opened.device, surface, size).await?;
        log::info!("drawing to a {:?} surface", config.format);
        Ok(Self {
            adapter: opened.adapter,
            device: opened.device,
            queue: opened.queue,
            config,
            loss,
        })
    }

    fn viewport(&self) -> ViewportRenderer {
        let sample_count = gpu::sample_count(
            &self.adapter,
            &self.device,
            self.config.format,
            DEPTH_FORMAT,
        );
        log::info!("drawing the viewport with {sample_count}x multisampling");
        ViewportRenderer::new(&self.device, self.config.format, sample_count)
    }

    fn largest_side(&self) -> u32 {
        self.device.limits().max_texture_dimension_2d
    }
}

async fn configure(
    adapter: &wgpu::Adapter,
    device: &wgpu::Device,
    surface: &wgpu::Surface<'static>,
    size: SurfaceSize,
) -> Result<wgpu::SurfaceConfiguration, RenderError> {
    let size = clamp_size(size, device.limits().max_texture_dimension_2d);
    let mut config = surface
        .get_default_config(adapter, size.width, size.height)
        .ok_or(RenderError::UnsupportedSurface)?;
    if let Some(format) = preferred_format(&surface.get_capabilities(adapter).formats) {
        config.format = format;
    }
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    surface.configure(device, &config);
    match scope.pop().await {
        None => Ok(config),
        Some(error) => {
            log::warn!("the drawing surface could not be configured: {error}");
            Err(RenderError::ConfigureSurface)
        }
    }
}

pub struct Renderer {
    window: Arc<dyn WindowTarget>,
    wake: Wake,
    instance: wgpu::Instance,
    surface: wgpu::Surface<'static>,
    gpu: Gpu,
    needs_reconfigure: bool,
    viewport: ViewportRenderer,
    generation: u64,
    pick_dropped: bool,
}

impl Renderer {
    pub async fn new(
        window: Arc<dyn WindowTarget>,
        size: SurfaceSize,
        wake: Wake,
    ) -> Result<Self, RenderError> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle_from_env(
                Box::new(Arc::clone(&window)),
            ));
        let surface = instance.create_surface(Arc::clone(&window))?;
        let gpu = Gpu::open(&instance, &surface, size, &wake).await?;
        let viewport = gpu.viewport();

        Ok(Self {
            window,
            wake,
            instance,
            surface,
            gpu,
            needs_reconfigure: false,
            viewport,
            generation: 0,
            pick_dropped: false,
        })
    }

    pub fn device(&self) -> &wgpu::Device {
        &self.gpu.device
    }

    pub fn queue(&self) -> &wgpu::Queue {
        &self.gpu.queue
    }

    pub fn format(&self) -> wgpu::TextureFormat {
        self.gpu.config.format
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn size(&self) -> SurfaceSize {
        SurfaceSize {
            width: self.gpu.config.width,
            height: self.gpu.config.height,
        }
    }

    pub fn resize(&mut self, size: SurfaceSize) {
        let size = clamp_size(size, self.gpu.largest_side());
        self.gpu.config.width = size.width;
        self.gpu.config.height = size.height;
        self.surface.configure(&self.gpu.device, &self.gpu.config);
        self.needs_reconfigure = false;
    }

    pub fn begin_frame(
        &mut self,
        window_size: SurfaceSize,
        viewport: Option<&ViewportFrame<'_>>,
    ) -> Result<FrameStart, RenderError> {
        if self.gpu.loss.is_lost() {
            self.recover()?;
        }
        self.viewport.picking().abandon_unsubmitted();
        if self.needs_reconfigure || clamp_size(window_size, self.gpu.largest_side()) != self.size()
        {
            self.resize(window_size);
        }

        let surface_texture = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(texture) => texture,
            wgpu::CurrentSurfaceTexture::Suboptimal(texture) => {
                self.needs_reconfigure = true;
                texture
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(FrameStart::Hidden);
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.needs_reconfigure = true;
                return Ok(FrameStart::Skipped);
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                self.recreate_surface()?;
                return Ok(FrameStart::Skipped);
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err(RenderError::SurfaceValidation);
            }
        };

        let view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });
        self.viewport.draw(
            &self.gpu.device,
            &self.gpu.queue,
            &mut encoder,
            &SurfaceTarget {
                view: &view,
                width: surface_texture.texture.width(),
                height: surface_texture.texture.height(),
            },
            viewport,
        );

        Ok(FrameStart::Ready(Box::new(Frame {
            surface_texture,
            generation: self.generation,
            view,
            encoder,
        })))
    }

    pub fn submit(
        &mut self,
        frame: Frame,
        preceding: impl IntoIterator<Item = wgpu::CommandBuffer>,
    ) {
        if frame.generation != self.generation || self.gpu.loss.is_lost() {
            log::warn!("dropping a frame drawn on a graphics device that was lost");
            return;
        }
        self.gpu
            .queue
            .submit(preceding.into_iter().chain([frame.encoder.finish()]));
        self.gpu.queue.present(frame.surface_texture);
        self.viewport.picking().after_submit();
    }

    pub fn poll_pick(&mut self) -> PickPoll {
        if std::mem::take(&mut self.pick_dropped) {
            return PickPoll::Failed;
        }
        self.viewport.picking().poll(&self.gpu.device)
    }

    pub fn is_pick_pending(&self) -> bool {
        self.viewport.is_pick_pending()
    }

    fn recover(&mut self) -> Result<(), RenderError> {
        log::warn!("opening a new graphics device to replace the lost one");
        let size = self.size();
        let reused = pollster::block_on(Gpu::open(&self.instance, &self.surface, size, &self.wake));
        let gpu = match reused {
            Ok(gpu) => gpu,
            Err(error) => {
                log::warn!(
                    "the window's surface could not be reused ({error}), creating a new one"
                );
                let surface = self.instance.create_surface(Arc::clone(&self.window))?;
                let gpu =
                    pollster::block_on(Gpu::open(&self.instance, &surface, size, &self.wake))?;
                self.surface = surface;
                gpu
            }
        };
        self.pick_dropped |= self.viewport.is_pick_pending();
        self.viewport = gpu.viewport();
        self.gpu = gpu;
        self.generation = self.generation.wrapping_add(1);
        self.needs_reconfigure = false;
        Ok(())
    }

    fn recreate_surface(&mut self) -> Result<(), RenderError> {
        log::warn!("drawing surface was lost, recreating it");
        let surface = self.instance.create_surface(Arc::clone(&self.window))?;
        if !surface
            .get_capabilities(&self.gpu.adapter)
            .formats
            .contains(&self.viewport.format())
        {
            return Err(RenderError::UnsupportedSurface);
        }
        self.surface = surface;
        self.resize(self.size());
        Ok(())
    }
}

const PREFERRED_FORMATS: [wgpu::TextureFormat; 2] = [
    wgpu::TextureFormat::Bgra8Unorm,
    wgpu::TextureFormat::Rgba8Unorm,
];

fn preferred_format(formats: &[wgpu::TextureFormat]) -> Option<wgpu::TextureFormat> {
    PREFERRED_FORMATS
        .into_iter()
        .find(|format| formats.contains(format))
        .or_else(|| formats.iter().copied().find(|format| !format.is_srgb()))
}

fn clamp_size(size: SurfaceSize, largest_side: u32) -> SurfaceSize {
    let largest_side = largest_side.max(1);
    SurfaceSize {
        width: size.width.clamp(1, largest_side),
        height: size.height.clamp(1, largest_side),
    }
}

#[cfg(test)]
mod tests {
    use wgpu::TextureFormat;

    use super::*;

    #[test]
    fn the_surface_prefers_plain_eight_bit_formats() {
        assert_eq!(
            preferred_format(&[
                TextureFormat::Rgba16Float,
                TextureFormat::Bgra8UnormSrgb,
                TextureFormat::Rgba8Unorm,
                TextureFormat::Bgra8Unorm,
            ]),
            Some(TextureFormat::Bgra8Unorm)
        );
        assert_eq!(
            preferred_format(&[TextureFormat::Rgba16Float, TextureFormat::Rgba8Unorm]),
            Some(TextureFormat::Rgba8Unorm)
        );
        assert_eq!(
            preferred_format(&[TextureFormat::Bgra8UnormSrgb, TextureFormat::Rgb10a2Unorm]),
            Some(TextureFormat::Rgb10a2Unorm)
        );
        assert_eq!(preferred_format(&[TextureFormat::Bgra8UnormSrgb]), None);
    }

    #[test]
    fn the_surface_never_passes_the_largest_texture_the_device_allows() {
        let wide = SurfaceSize {
            width: 10_240,
            height: 2_160,
        };

        assert_eq!(
            clamp_size(wide, 8_192),
            SurfaceSize {
                width: 8_192,
                height: 2_160,
            }
        );
        assert_eq!(clamp_size(wide, 16_384), wide);
        assert_eq!(
            clamp_size(
                SurfaceSize {
                    width: 0,
                    height: 0,
                },
                0
            ),
            SurfaceSize {
                width: 1,
                height: 1,
            }
        );
    }
}
