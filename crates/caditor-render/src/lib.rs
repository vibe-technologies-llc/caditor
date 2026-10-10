#![recursion_limit = "256"]

mod by_mesh;
mod camera;
mod culling;
mod gpu;
mod image;
mod kept;
mod lines;
mod mesh;
mod offscreen;
#[cfg(test)]
mod offscreen_tests;
mod picking;
mod scene;
mod settings;
mod silhouette;
mod styles;
mod through;
mod viewport;

use std::{
    fmt::Debug,
    sync::{Arc, mpsc},
    thread,
};

use wgpu::rwh::{HasDisplayHandle, HasWindowHandle};

pub use crate::{
    camera::{Camera, Projection, ProjectionMode, View, Viewpoint},
    gpu::Wake,
    image::{Background, ImageBands, ImageError, ImageRequest, MAX_IMAGE_SIDE},
    mesh::{
        Corner, Division, FaceStyle, MeshFace, MeshInstance, MeshPoint, MeshSource, Piece,
        ShadedMesh,
    },
    offscreen::OffscreenRenderer,
    picking::PickPoll,
    scene::{
        Batch, Color, CutFace, Fill, Grid, Layer, Line, MAX_SECTION_PLANES, Marker, PickHit,
        PickId, PickResult, Reflection, Scene, SectionPlane, Stroke, ViewportRect, is_cut_away,
        kept_span, section_slack,
    },
    settings::{AdapterPreference, GraphicsInfo, GraphicsSettings, Msaa, Shading},
    silhouette::Silhouette,
    viewport::{BACKGROUND, SurfaceTarget, ViewportFrame, ViewportRenderer, grid_minor_spacing},
};
use crate::{
    gpu::DeviceLoss,
    image::{ChannelOrder, IMAGE_FORMAT, ImageGpu, ImageTiles, TILE_SIDE},
    viewport::{DEPTH_FORMAT, Faults},
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
    #[error("could not start a thread to open a new graphics device: {0}")]
    RecoveryThread(std::io::Error),
    #[error("the thread opening a new graphics device ended without an answer")]
    RecoveryInterrupted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RenderFault {
    #[error(
        "The graphics device ran out of memory for the viewport at {from}x anti-aliasing, so it now uses {to}x."
    )]
    MultisamplingReduced { from: u32, to: u32 },
    #[error(
        "The graphics device could not provide memory for the viewport, so the 3D view stays blank. Close other programs that use the graphics card or make the window smaller."
    )]
    ViewportRefused,
    #[error(
        "The graphics device ran out of memory, so {meshes} bodies and {batches} sets of lines are not drawn. Hide some bodies or close other programs that use the graphics card."
    )]
    GeometryRefused { meshes: u32, batches: u32 },
    #[error(
        "The graphics device refused the memory used to tell what is under the cursor, so hovering and selecting in the 3D view may not work."
    )]
    PickingRefused,
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

struct Recovered {
    gpu: Gpu,
    surface: Option<Arc<wgpu::Surface<'static>>>,
}

struct Reopening {
    instance: wgpu::Instance,
    surface: Arc<wgpu::Surface<'static>>,
    window: Arc<dyn WindowTarget>,
    wake: Wake,
    size: SurfaceSize,
    vsync: bool,
    adapter: AdapterPreference,
}

impl Reopening {
    async fn run(&self) -> Result<Recovered, RenderError> {
        let reused = Gpu::open(
            &self.instance,
            &self.surface,
            self.size,
            &self.wake,
            self.vsync,
            self.adapter,
        )
        .await;
        match reused {
            Ok(gpu) => Ok(Recovered { gpu, surface: None }),
            Err(error) => {
                log::warn!(
                    "the window's surface could not be reused ({error}), creating a new one"
                );
                let surface = Arc::new(self.instance.create_surface(Arc::clone(&self.window))?);
                let gpu = Gpu::open(
                    &self.instance,
                    &surface,
                    self.size,
                    &self.wake,
                    self.vsync,
                    self.adapter,
                )
                .await?;
                Ok(Recovered {
                    gpu,
                    surface: Some(surface),
                })
            }
        }
    }
}

struct Gpu {
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    present_modes: Vec<wgpu::PresentMode>,
    loss: DeviceLoss,
    info: GraphicsInfo,
}

impl Gpu {
    async fn open(
        instance: &wgpu::Instance,
        surface: &wgpu::Surface<'static>,
        size: SurfaceSize,
        wake: &Wake,
        vsync: bool,
        adapter: AdapterPreference,
    ) -> Result<Self, RenderError> {
        let opened = gpu::open_device(instance, Some(surface), adapter).await?;
        let loss = DeviceLoss::watch(&opened.device, Arc::clone(wake));
        let present_modes = surface.get_capabilities(&opened.adapter).present_modes;
        let config = configure(
            &opened.adapter,
            &opened.device,
            surface,
            size,
            settings::present_mode(vsync, &present_modes),
        )
        .await?;
        log::info!(
            "drawing to a {:?} surface presented with {:?}",
            config.format,
            config.present_mode
        );
        let mut info = GraphicsInfo::describe(&opened.adapter);
        info.msaa_offered =
            gpu::offered_msaa(&opened.adapter, &opened.device, config.format, DEPTH_FORMAT);
        let mut gpu = Self {
            adapter: opened.adapter,
            device: opened.device,
            queue: opened.queue,
            config,
            present_modes,
            loss,
            info,
        };
        gpu.note_presentation();
        Ok(gpu)
    }

    fn viewport(&mut self, graphics: GraphicsSettings) -> ViewportRenderer {
        let msaa = self.msaa_for(graphics.msaa);
        log::info!(
            "drawing the viewport with {}x multisampling",
            msaa.samples()
        );
        let mut viewport = ViewportRenderer::new(&self.device, self.config.format, msaa.samples());
        viewport.set_shading(graphics.shading);
        viewport.set_linear_resolve(gpu::resolves_linearly(&self.adapter));
        viewport
    }

    fn msaa_for(&mut self, wanted: Msaa) -> Msaa {
        let msaa = wanted.closest(&self.info.msaa_offered);
        if msaa != wanted {
            log::info!(
                "{}x multisampling is not offered, using {}x",
                wanted.samples(),
                msaa.samples()
            );
        }
        self.info.msaa = msaa;
        msaa
    }

    fn present_with(&mut self, vsync: bool) {
        self.config.present_mode = settings::present_mode(vsync, &self.present_modes);
        self.note_presentation();
    }

    fn note_presentation(&mut self) {
        self.info.vsync_optional = settings::vsync_optional(&self.present_modes);
        self.info.vsync = self.config.present_mode == wgpu::PresentMode::Fifo;
    }

    fn largest_side(&self) -> u32 {
        self.device.limits().max_texture_dimension_2d
    }

    fn image_viewport(&self, shading: Shading) -> ViewportRenderer {
        let offered = gpu::offered_msaa(&self.adapter, &self.device, IMAGE_FORMAT, DEPTH_FORMAT);
        let msaa = self.info.msaa.closest(&offered);
        let mut viewport = ViewportRenderer::new(&self.device, IMAGE_FORMAT, msaa.samples());
        viewport.set_shading(shading);
        viewport.set_linear_resolve(gpu::resolves_linearly(&self.adapter));
        viewport
    }
}

async fn configure(
    adapter: &wgpu::Adapter,
    device: &wgpu::Device,
    surface: &wgpu::Surface<'static>,
    size: SurfaceSize,
    present_mode: wgpu::PresentMode,
) -> Result<wgpu::SurfaceConfiguration, RenderError> {
    let size = clamp_size(size, device.limits().max_texture_dimension_2d);
    let mut config = surface
        .get_default_config(adapter, size.width, size.height)
        .ok_or(RenderError::UnsupportedSurface)?;
    if let Some(format) = preferred_format(&surface.get_capabilities(adapter).formats) {
        config.format = format;
    }
    config.present_mode = present_mode;
    try_configure(device, surface, &config)
        .await
        .map(|()| config)
        .map_err(|_| RenderError::ConfigureSurface)
}

async fn try_configure(
    device: &wgpu::Device,
    surface: &wgpu::Surface<'static>,
    config: &wgpu::SurfaceConfiguration,
) -> Result<(), wgpu::Error> {
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    surface.configure(device, config);
    match scope.pop().await {
        None => Ok(()),
        Some(error) => {
            log::warn!(
                "the drawing surface could not be configured as {:?}: {error}",
                config.format
            );
            Err(error)
        }
    }
}

pub struct Renderer {
    window: Arc<dyn WindowTarget>,
    wake: Wake,
    instance: wgpu::Instance,
    surface: Arc<wgpu::Surface<'static>>,
    gpu: Gpu,
    graphics: GraphicsSettings,
    needs_reconfigure: bool,
    viewport: ViewportRenderer,
    generation: u64,
    pick_dropped: bool,
    image: Option<ImageTiles>,
    recovery: Option<mpsc::Receiver<Result<Recovered, RenderError>>>,
    validation_failures: u32,
    faults: Vec<RenderFault>,
}

impl Renderer {
    pub async fn new(
        window: Arc<dyn WindowTarget>,
        size: SurfaceSize,
        wake: Wake,
        graphics: GraphicsSettings,
    ) -> Result<Self, RenderError> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle_from_env(
                Box::new(Arc::clone(&window)),
            ));
        let surface = Arc::new(instance.create_surface(Arc::clone(&window))?);
        let mut gpu = Gpu::open(
            &instance,
            &surface,
            size,
            &wake,
            graphics.vsync,
            graphics.adapter,
        )
        .await?;
        let viewport = gpu.viewport(graphics);

        Ok(Self {
            window,
            wake,
            instance,
            surface,
            gpu,
            graphics,
            needs_reconfigure: false,
            viewport,
            generation: 0,
            pick_dropped: false,
            image: None,
            recovery: None,
            validation_failures: 0,
            faults: Vec::new(),
        })
    }

    pub fn take_faults(&mut self) -> Vec<RenderFault> {
        std::mem::take(&mut self.faults)
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

    pub fn is_uploading(&self) -> bool {
        self.viewport.is_uploading()
    }

    pub fn size(&self) -> SurfaceSize {
        SurfaceSize {
            width: self.gpu.config.width,
            height: self.gpu.config.height,
        }
    }

    pub fn graphics_info(&self) -> &GraphicsInfo {
        &self.gpu.info
    }

    pub fn set_graphics(&mut self, graphics: GraphicsSettings) {
        if graphics == self.graphics {
            return;
        }
        let adapter_changed = graphics.adapter != self.graphics.adapter;
        if graphics.vsync != self.graphics.vsync {
            self.gpu.present_with(graphics.vsync);
            self.needs_reconfigure = true;
        }
        if graphics.msaa != self.graphics.msaa {
            let msaa = self.gpu.msaa_for(graphics.msaa);
            self.viewport
                .set_sample_count(&self.gpu.device, msaa.samples());
        }
        self.viewport.set_shading(graphics.shading);
        self.graphics = graphics;
        if adapter_changed && let Err(error) = self.start_recovery() {
            log::warn!("could not open the graphics adapter that was asked for: {error}");
        }
    }

    pub fn set_background(&mut self, background: wgpu::Color) {
        self.viewport.set_background(background);
    }

    pub fn resize(&mut self, size: SurfaceSize) {
        let size = clamp_size(size, self.gpu.largest_side());
        self.gpu.config.width = size.width;
        self.gpu.config.height = size.height;
        if self.recovery.is_some() {
            return;
        }
        self.surface.configure(&self.gpu.device, &self.gpu.config);
        self.needs_reconfigure = false;
    }

    pub fn begin_frame(
        &mut self,
        window_size: SurfaceSize,
        viewport: Option<&ViewportFrame<'_>>,
    ) -> Result<FrameStart, RenderError> {
        if self.gpu.loss.is_lost() {
            self.start_recovery()?;
        }
        if !self.finish_recovery()? {
            return Ok(FrameStart::Skipped);
        }
        self.viewport.abandon_unsubmitted();
        if self.needs_reconfigure || clamp_size(window_size, self.gpu.largest_side()) != self.size()
        {
            self.resize(window_size);
        }

        let surface_texture = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(texture) => {
                self.validation_failures = 0;
                texture
            }
            wgpu::CurrentSurfaceTexture::Suboptimal(texture) => {
                self.validation_failures = 0;
                self.needs_reconfigure = true;
                texture
            }
            wgpu::CurrentSurfaceTexture::Timeout => {
                log::debug!("acquiring the next surface texture timed out, skipping the frame");
                return Ok(FrameStart::Skipped);
            }
            wgpu::CurrentSurfaceTexture::Occluded => {
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
                self.heal_surface()?;
                return Err(RenderError::SurfaceValidation);
            }
        };

        let view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let target = SurfaceTarget {
            view: &view,
            width: surface_texture.texture.width(),
            height: surface_texture.texture.height(),
        };
        let mut encoder = self.new_frame_encoder();
        let mut faults = self.viewport.draw(
            &self.gpu.device,
            &self.gpu.queue,
            &mut encoder,
            &target,
            viewport,
        );
        while faults.targets && self.step_multisampling_down() {
            encoder = self.new_frame_encoder();
            faults = self.viewport.draw(
                &self.gpu.device,
                &self.gpu.queue,
                &mut encoder,
                &target,
                viewport,
            );
        }
        self.report(faults);

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
        let submission = self
            .gpu
            .queue
            .submit(preceding.into_iter().chain([frame.encoder.finish()]));
        self.gpu.queue.present(frame.surface_texture);
        self.viewport
            .after_submit(&self.gpu.device, &self.wake, submission);
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

    pub fn is_pick_answered(&mut self) -> bool {
        self.pick_dropped
            || self.gpu.loss.is_lost()
            || self.viewport.picking().is_answered(&self.gpu.device)
    }

    pub fn render_image(&mut self, request: &ImageRequest<'_>) -> Result<ImageBands, ImageError> {
        if self.image.is_some() {
            return Err(ImageError::Busy);
        }
        let gpu = &self.gpu;
        let viewport = &self.viewport;
        let shading = self.graphics.shading;
        let (mut tiles, bands) = image::start(
            ImageGpu {
                device: gpu.device.clone(),
                queue: gpu.queue.clone(),
                loss: gpu.loss.clone(),
            },
            || match ChannelOrder::of(viewport.format()) {
                Some(_) => viewport.image_sibling(&gpu.device),
                None => {
                    let mut image = gpu.image_viewport(shading);
                    image.set_background(viewport.background());
                    image
                }
            },
            request,
            TILE_SIDE.min(gpu.largest_side()),
            Arc::clone(&self.wake),
        )?;
        tiles.advance();
        self.image = (!tiles.is_finished()).then_some(tiles);
        Ok(bands)
    }

    pub fn advance_image(&mut self) {
        if let Some(tiles) = self.image.as_mut() {
            tiles.advance();
            if tiles.is_finished() {
                self.image = None;
            }
        }
    }

    fn new_frame_encoder(&self) -> wgpu::CommandEncoder {
        self.gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            })
    }

    fn step_multisampling_down(&mut self) -> bool {
        let current = self.gpu.info.msaa;
        let lower = self
            .gpu
            .info
            .msaa_offered
            .iter()
            .copied()
            .filter(|level| *level < current)
            .max()
            .unwrap_or(Msaa::Off);
        if lower >= current {
            return false;
        }
        log::warn!(
            "lowering multisampling from {}x to {}x after the graphics device refused the viewport targets",
            current.samples(),
            lower.samples()
        );
        self.gpu.info.msaa = lower;
        self.viewport
            .set_sample_count(&self.gpu.device, lower.samples());
        self.faults.push(RenderFault::MultisamplingReduced {
            from: current.samples(),
            to: lower.samples(),
        });
        true
    }

    fn report(&mut self, faults: Faults) {
        if faults.targets {
            self.faults.push(RenderFault::ViewportRefused);
        }
        if faults.meshes > 0 || faults.batches > 0 {
            self.faults.push(RenderFault::GeometryRefused {
                meshes: faults.meshes,
                batches: faults.batches,
            });
        }
        if faults.picking {
            self.faults.push(RenderFault::PickingRefused);
        }
    }

    fn heal_surface(&mut self) -> Result<(), RenderError> {
        self.validation_failures = self.validation_failures.saturating_add(1);
        match self.validation_failures {
            1 => self.needs_reconfigure = true,
            2 => self.recreate_surface()?,
            3 => self.configure_conservatively(),
            _ => {
                self.validation_failures = 0;
                self.start_recovery()?;
            }
        }
        Ok(())
    }

    fn configure_conservatively(&mut self) {
        log::warn!(
            "the drawing surface keeps failing validation, configuring it conservatively with Fifo presentation"
        );
        let config = &mut self.gpu.config;
        config.present_mode = wgpu::PresentMode::Fifo;
        config.alpha_mode = wgpu::CompositeAlphaMode::Auto;
        config.desired_maximum_frame_latency = CONSERVATIVE_FRAME_LATENCY;
        self.gpu.note_presentation();
        self.resize(self.size());
    }

    fn start_recovery(&mut self) -> Result<(), RenderError> {
        if self.recovery.is_some() {
            return Ok(());
        }
        log::warn!("opening a new graphics device to replace the lost one");
        let reopening = Reopening {
            instance: self.instance.clone(),
            surface: Arc::clone(&self.surface),
            window: Arc::clone(&self.window),
            wake: Arc::clone(&self.wake),
            size: self.size(),
            vsync: self.graphics.vsync,
            adapter: self.graphics.adapter,
        };
        let (sender, receiver) = mpsc::channel();
        thread::Builder::new()
            .name("graphics-recovery".into())
            .spawn(move || {
                let outcome = pollster::block_on(reopening.run());
                let _ = sender.send(outcome);
                (reopening.wake)();
            })
            .map_err(RenderError::RecoveryThread)?;
        self.recovery = Some(receiver);
        Ok(())
    }

    fn finish_recovery(&mut self) -> Result<bool, RenderError> {
        let Some(receiver) = &self.recovery else {
            return Ok(true);
        };
        let outcome = match receiver.try_recv() {
            Ok(outcome) => outcome,
            Err(mpsc::TryRecvError::Empty) => return Ok(false),
            Err(mpsc::TryRecvError::Disconnected) => Err(RenderError::RecoveryInterrupted),
        };
        self.recovery = None;
        let recovered = outcome?;
        if let Some(surface) = recovered.surface {
            self.surface = surface;
        }
        let mut gpu = recovered.gpu;
        self.pick_dropped |= self.viewport.is_pick_pending();
        let background = self.viewport.background();
        self.viewport = gpu.viewport(self.graphics);
        self.viewport.set_background(background);
        self.gpu = gpu;
        self.generation = self.generation.wrapping_add(1);
        self.needs_reconfigure = false;
        self.validation_failures = 0;
        Ok(true)
    }

    fn recreate_surface(&mut self) -> Result<(), RenderError> {
        log::warn!("drawing surface was lost, recreating it");
        let surface = Arc::new(self.instance.create_surface(Arc::clone(&self.window))?);
        let capabilities = surface.get_capabilities(&self.gpu.adapter);
        if !capabilities.formats.contains(&self.viewport.format()) {
            return Err(RenderError::UnsupportedSurface);
        }
        self.surface = surface;
        self.gpu.present_modes = capabilities.present_modes;
        self.gpu.present_with(self.graphics.vsync);
        self.resize(self.size());
        Ok(())
    }
}

const CONSERVATIVE_FRAME_LATENCY: u32 = 2;

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
