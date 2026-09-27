use caditor_geometry::{Point3, Vector3};
use glam::DVec2;

use crate::{
    camera::{View, Viewpoint},
    scene::{Color, Fill, Layer, Line, Marker, PickId, PickResult, Scene, ViewportRect},
    viewport::{SurfaceTarget, ViewportFrame, ViewportRenderer},
};

const SIZE: u32 = 200;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const ROW_PITCH: u32 = 1024;
const LINE_COLOR: Color = Color::from_rgb8(250, 20, 20);

fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .ok()?;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()
}

fn scene() -> Scene {
    Scene {
        lines: vec![Line {
            start: Point3::new(-20.0, 0.0, 0.0),
            end: Point3::new(20.0, 0.0, 0.0),
            color: LINE_COLOR,
            width: 3.0,
            layer: Layer::Model,
            pick: PickId::from_index(0),
        }],
        fills: vec![Fill {
            convex_outline: vec![
                Point3::new(-30.0, -30.0, 0.0),
                Point3::new(30.0, -30.0, 0.0),
                Point3::new(30.0, 30.0, 0.0),
                Point3::new(-30.0, 30.0, 0.0),
            ],
            color: Color::from_rgba8(0, 0, 255, 40),
            pick: PickId::from_index(1),
        }],
        markers: vec![Marker {
            position: Point3::new(10.0, 10.0, 0.0),
            color: Color::from_rgb8(255, 255, 255),
            diameter: 7.0,
            layer: Layer::Model,
            pick: PickId::from_index(2),
        }],
        grid: None,
    }
}

struct Rendered {
    pick: PickResult,
    pixels: Vec<u8>,
}

fn render(device: &wgpu::Device, queue: &wgpu::Queue, view: &View, pick_at: DVec2) -> Rendered {
    let mut renderer = ViewportRenderer::new(device, FORMAT, 4);
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("offscreen target"),
        size: wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("offscreen readback"),
        size: u64::from(ROW_PITCH * SIZE),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let scene = scene();
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    renderer.draw(
        device,
        queue,
        &mut encoder,
        &SurfaceTarget {
            view: &target_view,
            width: SIZE,
            height: SIZE,
        },
        Some(&ViewportFrame {
            rect: ViewportRect {
                x: 0.0,
                y: 0.0,
                width: SIZE as f32,
                height: SIZE as f32,
            },
            view,
            scene: &scene,
            pick_at: Some(pick_at),
        }),
    );
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(ROW_PITCH),
                rows_per_image: Some(SIZE),
            },
        },
        target.size(),
    );
    queue.submit([encoder.finish()]);
    renderer.picking().after_submit();
    readback.map_async(wgpu::MapMode::Read, .., |result| result.unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();

    let pick = renderer.picking().poll(device).unwrap();
    let pixels = readback.get_mapped_range(..).unwrap().to_vec();
    Rendered { pick, pixels }
}

fn pixel(rendered: &Rendered, at: DVec2) -> [u8; 4] {
    let offset = at.y as usize * ROW_PITCH as usize + at.x as usize * 4;
    rendered.pixels[offset..offset + 4].try_into().unwrap()
}

#[test]
fn draws_and_picks_a_line_over_a_fill() {
    let Some((device, queue)) = gpu() else {
        eprintln!("no graphics adapter available, skipping the offscreen test");
        return;
    };
    let viewpoint = Viewpoint::looking_from(Vector3::Z, Point3::ZERO, 100.0).unwrap();
    let view = View::new(viewpoint, f64::from(SIZE), f64::from(SIZE));
    let on_line = view.project(Point3::new(5.0, 0.0, 0.0)).unwrap();
    let off_line = view.project(Point3::new(5.0, 20.0, 0.0)).unwrap();

    let rendered = render(&device, &queue, &view, on_line);

    let nearest = rendered.pick.hits[0];
    assert_eq!(nearest.id, PickId::from_index(0).unwrap());
    assert_eq!(nearest.offset_px, 0.0);
    assert!(nearest.position.distance(Point3::new(5.0, 0.0, 0.0)) < 0.5);
    assert!(
        rendered
            .pick
            .hits
            .iter()
            .any(|hit| hit.id == PickId::from_index(1).unwrap())
    );
    assert!(
        rendered
            .pick
            .hits
            .iter()
            .all(|hit| hit.id != PickId::from_index(2).unwrap())
    );

    let [red, green, blue, _] = pixel(&rendered, on_line);
    assert!(
        red > 200 && green < 80 && blue < 80,
        "line pixel was {red} {green} {blue}"
    );
    let [red, _, blue, _] = pixel(&rendered, off_line);
    assert!(blue > red, "fill pixel was not tinted blue");

    let rendered = render(
        &device,
        &queue,
        &view,
        view.project(Point3::new(10.0, 12.0, 0.0)).unwrap(),
    );
    let marker = rendered
        .pick
        .hits
        .iter()
        .find(|hit| hit.id == PickId::from_index(2).unwrap())
        .unwrap();
    assert!(marker.offset_px > 0.0 && marker.offset_px < 7.5);
}
