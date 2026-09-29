use std::{collections::BTreeMap, sync::Arc, time::Duration};

use caditor_render::{Frame, Renderer};
use egui::{ColorImage, ImageData, TextureId, TextureOptions, epaint::ImageDelta};
use egui_wgpu::ScreenDescriptor;
use egui_winit::accesskit_winit;
use winit::{
    event_loop::{ActiveEventLoop, EventLoopProxy},
    window::Window,
};

const LONGEST_SCHEDULED_REPAINT: Duration = Duration::from_secs(3600);

pub struct UiFrame {
    primitives: Vec<egui::ClippedPrimitive>,
    textures: egui::TexturesDelta,
    pixels_per_point: f32,
    pub repaint_after: Option<Duration>,
}

pub struct Overlay {
    context: egui::Context,
    state: egui_winit::State,
    renderer: egui_wgpu::Renderer,
    generation: u64,
    textures: TextureMirror,
}

impl Overlay {
    pub fn new(window: &Window, renderer: &Renderer) -> Self {
        let context = egui::Context::default();
        let state = egui_winit::State::new(
            context.clone(),
            egui::ViewportId::ROOT,
            window,
            Some(window.scale_factor() as f32),
            window.theme(),
            Some(largest_texture_side(renderer)),
        );
        Self {
            context,
            state,
            renderer: egui_renderer(renderer),
            generation: renderer.generation(),
            textures: TextureMirror::default(),
        }
    }

    pub fn enable_accessibility<T: From<accesskit_winit::Event> + Send>(
        &mut self,
        event_loop: &ActiveEventLoop,
        window: &Window,
        proxy: EventLoopProxy<T>,
    ) {
        if window.is_visible() == Some(true) {
            log::warn!("screen readers are unavailable: the window was shown too early");
            return;
        }
        self.state.init_accesskit(event_loop, window, proxy);
    }

    pub fn on_accessibility_event(&mut self, event: accesskit_winit::WindowEvent) {
        match event {
            accesskit_winit::WindowEvent::InitialTreeRequested => {
                self.context.enable_accesskit();
            }
            accesskit_winit::WindowEvent::ActionRequested(request) => {
                self.state.on_accesskit_action_request(request);
            }
            accesskit_winit::WindowEvent::AccessibilityDeactivated => {
                self.context.disable_accesskit();
            }
        }
    }

    pub fn on_window_event(&mut self, window: &Window, event: &winit::event::WindowEvent) -> bool {
        self.state.on_window_event(window, event).repaint
    }

    pub fn run(&mut self, window: &Window, run_ui: impl FnMut(&mut egui::Ui)) -> UiFrame {
        let input = self.state.take_egui_input(window);
        let output = self.context.run_ui(input, run_ui);
        self.state
            .handle_platform_output(window, output.platform_output);

        let repaint_after = output
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .map(|viewport| viewport.repaint_delay)
            .filter(|delay| *delay < LONGEST_SCHEDULED_REPAINT);
        UiFrame {
            primitives: self
                .context
                .tessellate(output.shapes, output.pixels_per_point),
            textures: output.textures_delta,
            pixels_per_point: output.pixels_per_point,
            repaint_after,
        }
    }

    pub fn paint(
        &mut self,
        renderer: &Renderer,
        frame: Option<&mut Frame>,
        mut ui: UiFrame,
    ) -> Vec<wgpu::CommandBuffer> {
        if renderer.generation() != self.generation {
            self.follow_new_device(renderer);
        }
        for (id, deltas) in ui.textures.set.drain() {
            for delta in deltas {
                self.textures.apply(id, &delta);
                self.renderer
                    .update_texture(renderer.device(), renderer.queue(), id, &delta);
            }
        }

        let mut command_buffers = Vec::new();
        if let Some(frame) = frame {
            let size = renderer.size();
            let screen = ScreenDescriptor {
                size_in_pixels: [size.width, size.height],
                pixels_per_point: ui.pixels_per_point,
            };
            command_buffers = self.renderer.update_buffers(
                renderer.device(),
                renderer.queue(),
                &mut frame.encoder,
                &ui.primitives,
                &screen,
            );

            let mut pass = frame
                .encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("egui"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &frame.view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                })
                .forget_lifetime();
            self.renderer.render(&mut pass, &ui.primitives, &screen);
        }

        for id in ui.textures.free.drain() {
            self.textures.free(id);
            self.renderer.free_texture(&id);
        }
        command_buffers
    }

    fn follow_new_device(&mut self, renderer: &Renderer) {
        log::info!("drawing the interface on the new graphics device");
        self.renderer = egui_renderer(renderer);
        self.state
            .set_max_texture_side(largest_texture_side(renderer));
        for (id, delta) in self.textures.whole() {
            self.renderer
                .update_texture(renderer.device(), renderer.queue(), id, &delta);
        }
        self.generation = renderer.generation();
    }
}

fn egui_renderer(renderer: &Renderer) -> egui_wgpu::Renderer {
    egui_wgpu::Renderer::new(
        renderer.device(),
        renderer.format(),
        egui_wgpu::RendererOptions::default(),
    )
}

fn largest_texture_side(renderer: &Renderer) -> usize {
    renderer.device().limits().max_texture_dimension_2d as usize
}

#[derive(Debug, Default)]
struct TextureMirror {
    images: BTreeMap<TextureId, (Arc<ColorImage>, TextureOptions)>,
}

impl TextureMirror {
    fn apply(&mut self, id: TextureId, delta: &ImageDelta) {
        let ImageData::Color(patch) = &delta.image;
        match delta.pos {
            None => {
                self.images.insert(id, (Arc::clone(patch), delta.options));
            }
            Some(position) => {
                if let Some((image, options)) = self.images.get_mut(&id) {
                    paste(Arc::make_mut(image), patch, position);
                    *options = delta.options;
                }
            }
        }
    }

    fn free(&mut self, id: TextureId) {
        self.images.remove(&id);
    }

    fn whole(&self) -> impl Iterator<Item = (TextureId, ImageDelta)> + '_ {
        self.images.iter().map(|(id, (image, options))| {
            (
                *id,
                ImageDelta::full(ImageData::Color(Arc::clone(image)), *options),
            )
        })
    }
}

fn paste(image: &mut ColorImage, patch: &ColorImage, [left, top]: [usize; 2]) {
    let [width, _] = image.size;
    let [patch_width, _] = patch.size;
    if patch_width == 0 {
        return;
    }
    let columns = patch_width.min(width.saturating_sub(left));
    for (row, source) in patch.pixels.chunks(patch_width).enumerate() {
        let start = (top + row) * width + left;
        let (Some(target), Some(source)) = (
            image.pixels.get_mut(start..start + columns),
            source.get(..columns),
        ) else {
            continue;
        };
        target.copy_from_slice(source);
    }
}

#[cfg(test)]
mod tests {
    use egui::Color32;

    use super::*;

    fn filled(size: [usize; 2], color: Color32) -> Arc<ColorImage> {
        Arc::new(ColorImage::filled(size, color))
    }

    #[test]
    fn the_mirror_keeps_every_texture_whole_for_a_new_device() {
        let font = TextureId::Managed(0);
        let freed = TextureId::Managed(1);
        let mut mirror = TextureMirror::default();

        mirror.apply(
            font,
            &ImageDelta::full(filled([4, 3], Color32::BLACK), TextureOptions::LINEAR),
        );
        mirror.apply(
            font,
            &ImageDelta::partial(
                [2, 1],
                filled([3, 2], Color32::WHITE),
                TextureOptions::LINEAR,
            ),
        );
        mirror.apply(
            freed,
            &ImageDelta::full(filled([1, 1], Color32::RED), TextureOptions::NEAREST),
        );
        mirror.free(freed);
        let whole: Vec<(TextureId, ImageDelta)> = mirror.whole().collect();

        assert_eq!(whole.len(), 1);
        let (id, delta) = &whole[0];
        let ImageData::Color(image) = &delta.image;
        assert_eq!(*id, font);
        assert_eq!(delta.pos, None);
        assert_eq!(image.size, [4, 3]);
        assert_eq!(image.pixels[0], Color32::BLACK);
        assert_eq!(image.pixels[4 + 1], Color32::BLACK);
        assert_eq!(image.pixels[4 + 2], Color32::WHITE);
        assert_eq!(image.pixels[4 + 3], Color32::WHITE);
        assert_eq!(image.pixels[8 + 2], Color32::WHITE);
    }
}
