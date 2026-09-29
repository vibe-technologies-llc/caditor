use std::sync::Arc;

use caditor_geometry::{Point3, Vector3};
use glam::DVec2;

use crate::{
    camera::{View, Viewpoint},
    gpu::{Bytes, GrowableBuffer},
    mesh::{FaceStyle, MeshFace, MeshInstance, MeshPoint, ShadedMesh},
    scene::{Color, Fill, Layer, Line, Marker, PickId, PickResult, Scene, ViewportRect},
    viewport::{SurfaceTarget, ViewportFrame, ViewportRenderer},
};

const SIZE: u32 = 200;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const ROW_PITCH: u32 = 1024;
const LINE_COLOR: Color = Color::from_rgb8(250, 20, 20);

const REQUIRE_GPU: &str = "CADITOR_REQUIRE_GPU";

fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
    let found = device();
    if found.is_none() {
        assert!(
            std::env::var_os(REQUIRE_GPU).is_none(),
            "no graphics adapter is available, and {REQUIRE_GPU} says the offscreen tests must run"
        );
        eprintln!("no graphics adapter available, skipping the offscreen test");
    }
    found
}

fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .ok()?;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()
}

fn scene() -> Scene {
    Scene {
        meshes: Vec::new(),
        lines: vec![Line {
            start: Point3::new(-20.0, 0.0, 0.0),
            end: Point3::new(20.0, 0.0, 0.0),
            color: LINE_COLOR,
            width: 3.0,
            layer: Layer::Model,
            pick: PickId::from_index(0),
        }],
        fills: vec![Fill::convex(
            &[
                Point3::new(-30.0, -30.0, 0.0),
                Point3::new(30.0, -30.0, 0.0),
                Point3::new(30.0, 30.0, 0.0),
                Point3::new(-30.0, 30.0, 0.0),
            ],
            Color::from_rgba8(0, 0, 255, 40),
            Layer::Reference,
            PickId::from_index(1),
        )],
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

fn render(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    view: &View,
    scene: &Scene,
    pick_at: DVec2,
) -> Rendered {
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
            scene,
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

    let crate::PickPoll::Ready(pick) = renderer.picking().poll(device) else {
        panic!("the pick should be read back");
    };
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
        return;
    };
    let viewpoint = Viewpoint::looking_from(Vector3::Z, Point3::ZERO, 100.0).unwrap();
    let view = View::new(viewpoint, f64::from(SIZE), f64::from(SIZE));
    let on_line = view.project(Point3::new(5.0, 0.0, 0.0)).unwrap();
    let off_line = view.project(Point3::new(5.0, 20.0, 0.0)).unwrap();

    let rendered = render(&device, &queue, &view, &scene(), on_line);

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
        &scene(),
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

fn box_mesh(half: f64) -> ShadedMesh {
    let face = |normal: Vector3| {
        let (u, v) = normal.any_orthonormal_pair();
        let corner = |a: f64, b: f64| MeshPoint {
            position: Point3::ZERO + (normal + u * a + v * b) * half,
            normal,
        };
        MeshFace {
            points: vec![
                corner(-1.0, -1.0),
                corner(1.0, -1.0),
                corner(1.0, 1.0),
                corner(-1.0, 1.0),
            ],
            triangles: vec![[0, 1, 2], [0, 2, 3]],
        }
    };
    ShadedMesh::new(
        [
            Vector3::X,
            Vector3::NEG_X,
            Vector3::Y,
            Vector3::NEG_Y,
            Vector3::Z,
            Vector3::NEG_Z,
        ]
        .map(face),
    )
}

#[test]
fn draws_shaded_faces_that_hide_what_is_behind_them_and_picks_the_face_in_front() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let mesh = Arc::new(box_mesh(20.0));
    let styles: Vec<FaceStyle> = (0..6)
        .map(|index| FaceStyle {
            color: Color::from_rgb8(40, 200, 40),
            pick: PickId::from_index(10 + index),
        })
        .collect();
    let scene = Scene {
        meshes: vec![MeshInstance {
            mesh,
            faces: styles,
        }],
        lines: vec![Line {
            start: Point3::new(-50.0, 0.0, 0.0),
            end: Point3::new(50.0, 0.0, 0.0),
            color: LINE_COLOR,
            width: 3.0,
            layer: Layer::Model,
            pick: PickId::from_index(0),
        }],
        ..Scene::default()
    };
    let viewpoint = Viewpoint::looking_from(Vector3::Z, Point3::ZERO, 150.0).unwrap();
    let view = View::new(viewpoint, f64::from(SIZE), f64::from(SIZE));
    let on_top = view.project(Point3::new(0.0, 10.0, 20.0)).unwrap();
    let hidden_line = view.project(Point3::new(5.0, 0.0, 20.0)).unwrap();
    let visible_line = view.project(Point3::new(40.0, 0.0, 0.0)).unwrap();

    let rendered = render(&device, &queue, &view, &scene, on_top);

    let nearest = rendered.pick.hits[0];
    assert_eq!(nearest.id, PickId::from_index(14).unwrap());
    assert_eq!(nearest.offset_px, 0.0);
    assert!(nearest.position.distance(Point3::new(0.0, 10.0, 20.0)) < 0.5);
    let [red, green, blue, _] = pixel(&rendered, on_top);
    assert!(
        green > 60 && green > red * 2 && green > blue * 2,
        "face pixel was {red} {green} {blue}"
    );
    let [red, green, _, _] = pixel(&rendered, hidden_line);
    assert!(
        green > red,
        "the line inside the box showed through its top"
    );
    let [red, green, _, _] = pixel(&rendered, visible_line);
    assert!(
        red > 200 && green < 80,
        "the line outside the box was hidden"
    );

    let rendered = render(&device, &queue, &view, &scene, hidden_line);
    assert!(
        rendered
            .pick
            .hits
            .iter()
            .all(|hit| hit.id != PickId::from_index(0).unwrap())
    );
}

#[test]
fn faces_that_cannot_be_picked_still_hide_what_is_behind_them_from_picking() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let scene = Scene {
        meshes: vec![MeshInstance {
            mesh: Arc::new(box_mesh(20.0)),
            faces: vec![
                FaceStyle {
                    color: Color::from_rgb8(90, 90, 90),
                    pick: None,
                };
                6
            ],
        }],
        lines: vec![Line {
            start: Point3::new(-50.0, 0.0, 0.0),
            end: Point3::new(50.0, 0.0, 0.0),
            color: LINE_COLOR,
            width: 3.0,
            layer: Layer::Model,
            pick: PickId::from_index(0),
        }],
        ..Scene::default()
    };
    let viewpoint = Viewpoint::looking_from(Vector3::Z, Point3::ZERO, 150.0).unwrap();
    let view = View::new(viewpoint, f64::from(SIZE), f64::from(SIZE));
    let hidden_line = view.project(Point3::new(5.0, 0.0, 20.0)).unwrap();
    let visible_line = view.project(Point3::new(40.0, 0.0, 0.0)).unwrap();

    let behind = render(&device, &queue, &view, &scene, hidden_line);
    let beside = render(&device, &queue, &view, &scene, visible_line);

    assert!(behind.pick.hits.is_empty(), "{:?}", behind.pick.hits);
    assert_eq!(beside.pick.hits[0].id, PickId::from_index(0).unwrap());
}

fn square_fill(z: f64, half: f64, layer: Layer, index: usize) -> Fill {
    Fill::convex(
        &[
            Point3::new(-half, -half, z),
            Point3::new(half, -half, z),
            Point3::new(half, half, z),
            Point3::new(-half, half, z),
        ],
        Color::from_rgba8(0, 0, 255, 40),
        layer,
        PickId::from_index(index),
    )
}

#[test]
fn reference_fills_are_picked_only_where_nothing_else_is() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let top_face = PickId::from_index(14).unwrap();
    let styles: Vec<FaceStyle> = (0..6)
        .map(|index| FaceStyle {
            color: Color::from_rgb8(40, 200, 40),
            pick: PickId::from_index(10 + index),
        })
        .collect();
    let scene = Scene {
        meshes: vec![MeshInstance {
            mesh: Arc::new(box_mesh(20.0)),
            faces: styles,
        }],
        fills: vec![
            square_fill(40.0, 60.0, Layer::Reference, 1),
            square_fill(30.0, 60.0, Layer::Reference, 2),
            square_fill(20.0, 5.0, Layer::Model, 3),
        ],
        ..Scene::default()
    };
    let viewpoint = Viewpoint::looking_from(Vector3::Z, Point3::ZERO, 150.0).unwrap();
    let view = View::new(viewpoint, f64::from(SIZE), f64::from(SIZE));
    let first_hit = |at: Point3| {
        render(&device, &queue, &view, &scene, view.project(at).unwrap())
            .pick
            .hits
            .first()
            .map(|hit| (hit.id, hit.position))
    };

    let (id, position) = first_hit(Point3::new(12.0, 12.0, 20.0)).unwrap();
    assert_eq!(id, top_face);
    assert!(position.distance(Point3::new(12.0, 12.0, 20.0)) < 0.5);

    let (id, _) = first_hit(Point3::new(0.0, 0.0, 20.0)).unwrap();
    assert_eq!(id, PickId::from_index(3).unwrap());

    let (id, position) = first_hit(Point3::new(45.0, 45.0, 40.0)).unwrap();
    assert_eq!(id, PickId::from_index(1).unwrap());
    assert!((position.z - 40.0).abs() < 0.5, "{position:?}");
}

#[test]
fn a_pick_whose_frame_was_never_submitted_fails_and_the_next_one_is_read() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let viewpoint = Viewpoint::looking_from(Vector3::Z, Point3::ZERO, 100.0).unwrap();
    let view = View::new(viewpoint, f64::from(SIZE), f64::from(SIZE));
    let on_line = view.project(Point3::new(5.0, 0.0, 0.0)).unwrap();
    let mut renderer = ViewportRenderer::new(&device, FORMAT, 4);
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
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let scene = scene();
    let draw = |renderer: &mut ViewportRenderer| {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        renderer.draw(
            &device,
            &queue,
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
                view: &view,
                scene: &scene,
                pick_at: Some(on_line),
            }),
        );
        encoder
    };

    drop(draw(&mut renderer));
    let pending = renderer.picking().poll(&device);
    renderer.picking().abandon_unsubmitted();
    let abandoned = renderer.picking().poll(&device);
    let encoder = draw(&mut renderer);
    queue.submit([encoder.finish()]);
    renderer.picking().after_submit();
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let read = renderer.picking().poll(&device);

    assert_eq!(pending, crate::PickPoll::Pending);
    assert_eq!(abandoned, crate::PickPoll::Failed);
    assert!(matches!(read, crate::PickPoll::Ready(pick) if !pick.hits.is_empty()));
}

#[test]
fn a_viewport_of_no_size_keeps_its_meshes_until_the_scene_drops_them() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let mut renderer = ViewportRenderer::new(&device, FORMAT, 4);
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
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let viewpoint = Viewpoint::looking_from(Vector3::Z, Point3::ZERO, 150.0).unwrap();
    let view = View::new(viewpoint, f64::from(SIZE), f64::from(SIZE));
    let mut draw = |scene: &Scene, size: f32| {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        renderer.draw(
            &device,
            &queue,
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
                    width: size,
                    height: size,
                },
                view: &view,
                scene,
                pick_at: None,
            }),
        );
        queue.submit([encoder.finish()]);
    };
    let mesh = Arc::new(box_mesh(20.0));
    let scene = Scene {
        meshes: vec![MeshInstance {
            mesh: Arc::clone(&mesh),
            faces: vec![
                FaceStyle {
                    color: Color::from_rgb8(40, 200, 40),
                    pick: None,
                };
                6
            ],
        }],
        ..Scene::default()
    };

    draw(&scene, SIZE as f32);
    drop(scene);
    assert_eq!(Arc::strong_count(&mesh), 2);

    draw(&Scene::default(), 0.0);
    assert_eq!(Arc::strong_count(&mesh), 2);

    draw(&Scene::default(), SIZE as f32);
    assert_eq!(Arc::strong_count(&mesh), 1);
}

#[test]
fn a_buffer_shrinks_back_once_it_has_stayed_mostly_empty_for_a_while() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let mut buffer = GrowableBuffer::new(&device, "test", wgpu::BufferUsages::VERTEX);
    let mut large = Bytes::default();
    large.floats(&[1.0; 100_000]);
    let mut small = Bytes::default();
    small.floats(&[1.0; 10]);

    buffer.upload(&device, &queue, &large);
    let grown = buffer.size();
    assert!(grown >= large.len());

    for _ in 0..GrowableBuffer::SHRINK_AFTER_UPLOADS - 1 {
        buffer.upload(&device, &queue, &small);
    }
    buffer.upload(&device, &queue, &large);
    assert_eq!(buffer.size(), grown);

    for _ in 0..GrowableBuffer::SHRINK_AFTER_UPLOADS {
        buffer.upload(&device, &queue, &small);
    }
    assert_eq!(buffer.size(), GrowableBuffer::INITIAL_SIZE);
}
