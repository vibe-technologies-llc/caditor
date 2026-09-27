use caditor_document::Document;
use caditor_render::{Frame, Renderer};
use egui_wgpu::ScreenDescriptor;
use winit::window::Window;

use crate::panels;

pub struct Paint {
    pub command_buffers: Vec<wgpu::CommandBuffer>,
    pub repaint_now: bool,
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

    pub fn paint(
        &mut self,
        window: &Window,
        renderer: &Renderer,
        frame: &mut Frame,
        document: &Document,
    ) -> Paint {
        let input = self.state.take_egui_input(window);
        let mut output = self.context.run_ui(input, |ui| panels::show(ui, document));
        self.state
            .handle_platform_output(window, output.platform_output);

        let primitives = self
            .context
            .tessellate(output.shapes, output.pixels_per_point);
        let size = renderer.size();
        let screen = ScreenDescriptor {
            size_in_pixels: [size.width, size.height],
            pixels_per_point: output.pixels_per_point,
        };

        for (id, deltas) in output.textures_delta.set.drain() {
            for delta in deltas {
                self.renderer
                    .update_texture(renderer.device(), renderer.queue(), id, &delta);
            }
        }
        let command_buffers = self.renderer.update_buffers(
            renderer.device(),
            renderer.queue(),
            &mut frame.encoder,
            &primitives,
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
        self.renderer.render(&mut pass, &primitives, &screen);
        drop(pass);

        for id in output.textures_delta.free.drain() {
            self.renderer.free_texture(&id);
        }

        let repaint_now = output
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .is_some_and(|viewport| viewport.repaint_delay.is_zero());

        Paint {
            command_buffers,
            repaint_now,
        }
    }
}
