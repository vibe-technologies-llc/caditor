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

use crate::viewport::{DEPTH_FORMAT, SurfaceTarget, ViewportRenderer};
pub use crate::{
    camera::{Camera, View, Viewpoint},
    mesh::{FaceStyle, MeshFace, MeshInstance, MeshPoint, ShadedMesh},
    scene::{
        Color, Fill, Grid, Layer, Line, Marker, PickHit, PickId, PickResult, Scene, ViewportRect,
    },
    viewport::ViewportFrame,
};

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("could not create a drawing surface for the window: {0}")]
    CreateSurface(#[from] wgpu::CreateSurfaceError),
    #[error("no compatible graphics adapter was found: {0}")]
    RequestAdapter(#[from] wgpu::RequestAdapterError),
    #[error("the graphics adapter refused to open a device: {0}")]
    RequestDevice(#[from] wgpu::RequestDeviceError),
    #[error("the graphics adapter cannot present to this window")]
    UnsupportedSurface,
    #[error("the window's drawing surface raised a validation error")]
    SurfaceValidation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SurfaceSize {
    pub width: u32,
    pub height: u32,
}

pub struct Frame {
    surface_texture: wgpu::SurfaceTexture,
    pub view: wgpu::TextureView,
    pub encoder: wgpu::CommandEncoder,
}

pub trait WindowTarget: HasWindowHandle + HasDisplayHandle + Debug + Send + Sync + 'static {}

impl<T> WindowTarget for T where
    T: HasWindowHandle + HasDisplayHandle + Debug + Send + Sync + 'static
{
}

pub struct Renderer {
    window: Arc<dyn WindowTarget>,
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    needs_reconfigure: bool,
    viewport: ViewportRenderer,
}

impl Renderer {
    pub async fn new(
        window: Arc<dyn WindowTarget>,
        size: SurfaceSize,
    ) -> Result<Self, RenderError> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle_from_env(
                Box::new(Arc::clone(&window)),
            ));
        let surface = instance.create_surface(Arc::clone(&window))?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await?;
        log::info!("using graphics adapter {:?}", adapter.get_info());

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("caditor"),
                ..Default::default()
            })
            .await?;
        device.on_uncaptured_error(Arc::new(|error| {
            log::error!("the graphics device reported an error: {error}");
        }));

        let size = clamp_size(size);
        let mut config = surface
            .get_default_config(&adapter, size.width, size.height)
            .ok_or(RenderError::UnsupportedSurface)?;
        let capabilities = surface.get_capabilities(&adapter);
        if let Some(format) = capabilities.formats.iter().find(|format| !format.is_srgb()) {
            config.format = *format;
        }
        surface.configure(&device, &config);
        let sample_count = supported_sample_count(&adapter, config.format);
        log::info!("drawing the viewport with {sample_count}x multisampling");
        let viewport = ViewportRenderer::new(&device, config.format, sample_count);

        Ok(Self {
            window,
            instance,
            adapter,
            surface,
            device,
            queue,
            config,
            needs_reconfigure: false,
            viewport,
        })
    }

    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    pub fn format(&self) -> wgpu::TextureFormat {
        self.config.format
    }

    pub fn size(&self) -> SurfaceSize {
        SurfaceSize {
            width: self.config.width,
            height: self.config.height,
        }
    }

    pub fn resize(&mut self, size: SurfaceSize) {
        let size = clamp_size(size);
        self.config.width = size.width;
        self.config.height = size.height;
        self.surface.configure(&self.device, &self.config);
        self.needs_reconfigure = false;
    }

    pub fn begin_frame(
        &mut self,
        viewport: Option<&ViewportFrame<'_>>,
    ) -> Result<Option<Frame>, RenderError> {
        if self.needs_reconfigure {
            self.resize(self.size());
        }

        let surface_texture = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(texture) => texture,
            wgpu::CurrentSurfaceTexture::Suboptimal(texture) => {
                self.needs_reconfigure = true;
                texture
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(None);
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.resize(self.size());
                return Ok(None);
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                self.recreate_surface()?;
                return Ok(None);
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err(RenderError::SurfaceValidation);
            }
        };

        let view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });
        self.viewport.draw(
            &self.device,
            &self.queue,
            &mut encoder,
            &SurfaceTarget {
                view: &view,
                width: surface_texture.texture.width(),
                height: surface_texture.texture.height(),
            },
            viewport,
        );

        Ok(Some(Frame {
            surface_texture,
            view,
            encoder,
        }))
    }

    pub fn submit(
        &mut self,
        frame: Frame,
        preceding: impl IntoIterator<Item = wgpu::CommandBuffer>,
    ) {
        self.queue
            .submit(preceding.into_iter().chain([frame.encoder.finish()]));
        self.queue.present(frame.surface_texture);
        self.viewport.picking().after_submit();
    }

    pub fn poll_pick(&mut self) -> Option<PickResult> {
        self.viewport.picking().poll(&self.device)
    }

    pub fn is_pick_pending(&self) -> bool {
        self.viewport.is_pick_pending()
    }

    fn recreate_surface(&mut self) -> Result<(), RenderError> {
        log::warn!("drawing surface was lost, recreating it");
        let surface = self.instance.create_surface(Arc::clone(&self.window))?;
        if !surface
            .get_capabilities(&self.adapter)
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

fn supported_sample_count(adapter: &wgpu::Adapter, format: wgpu::TextureFormat) -> u32 {
    let color = adapter.get_texture_format_features(format).flags;
    let depth = adapter.get_texture_format_features(DEPTH_FORMAT).flags;
    [4, 2]
        .into_iter()
        .find(|&count| {
            color.sample_count_supported(count)
                && depth.sample_count_supported(count)
                && color.contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_RESOLVE)
        })
        .unwrap_or(1)
}

fn clamp_size(size: SurfaceSize) -> SurfaceSize {
    SurfaceSize {
        width: size.width.max(1),
        height: size.height.max(1),
    }
}
