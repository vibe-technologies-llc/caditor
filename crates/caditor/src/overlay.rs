use std::time::Duration;

use caditor_render::{Frame, Renderer};
use egui_wgpu::ScreenDescriptor;
use winit::window::Window;

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
            Some(renderer.device().limits().max_texture_dimension_2d as usize),
        );
        let renderer = egui_wgpu::Renderer::new(
            renderer.device(),
            renderer.format(),
            egui_wgpu::RendererOptions::default(),
        );
        Self {
            context,
            state,
            renderer,
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
        for (id, deltas) in ui.textures.set.drain() {
            for delta in deltas {
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
            self.renderer.free_texture(&id);
        }
        command_buffers
    }
}
