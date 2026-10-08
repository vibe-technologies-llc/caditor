use std::sync::Arc;

use caditor_geometry::{Aabb, Point3, RigidTransform, Vector3};
use glam::Vec3;
use wgpu::util::DeviceExt;

use crate::{
    culling::{ClipWindow, placed_corners},
    gpu::{self, Bytes},
    scene::{Color, PickId},
    viewport::relative_to_eye,
};

pub const MESH_VERTEX_STRIDE: u64 = 28;
const INDEX_BYTES: u64 = 4;
const STYLE_BINDING: u32 = 1;
const PLACEMENT_BINDING: u32 = 2;
const PLACEMENT_BYTES: u64 = 80;
const STYLE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg32Uint;
const STYLE_TEXEL_BYTES: u32 = 8;
const UNSTYLED_FACE: FaceStyle = FaceStyle {
    color: Color::from_rgb8(160, 164, 172),
    pick: None,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeshPoint {
    pub position: Point3,
    pub normal: Vector3,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct MeshFace {
    pub points: Vec<MeshPoint>,
    pub triangles: Vec<[u32; 3]>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct GpuVertex {
    position: Vec3,
    normal: Vec3,
    face: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ShadedMesh {
    origin: Point3,
    bounds: Option<Aabb>,
    vertices: Vec<GpuVertex>,
    indices: Vec<u32>,
    face_count: usize,
}

impl ShadedMesh {
    pub fn new(faces: impl IntoIterator<Item = MeshFace>) -> Self {
        let faces: Vec<MeshFace> = faces.into_iter().collect();
        let bounds = Aabb::from_points(
            faces
                .iter()
                .flat_map(|face| face.points.iter().map(|point| point.position)),
        );
        let origin = bounds.map_or(Point3::ZERO, |bounds| bounds.center());
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        for (index, face) in faces.iter().enumerate() {
            let (Ok(face_index), Ok(first)) = (u32::try_from(index), u32::try_from(vertices.len()))
            else {
                break;
            };
            let count = face.points.len();
            vertices.extend(face.points.iter().map(|point| GpuVertex {
                position: relative_to_eye(point.position, origin),
                normal: point.normal.normalize_or_zero().as_vec3(),
                face: face_index,
            }));
            for triangle in &face.triangles {
                if triangle.iter().all(|corner| (*corner as usize) < count) {
                    indices.extend(triangle.map(|corner| first + corner));
                }
            }
        }
        Self {
            origin,
            bounds,
            vertices,
            indices,
            face_count: faces.len(),
        }
    }

    pub fn face_count(&self) -> usize {
        self.face_count
    }

    pub fn bounds(&self) -> Option<Aabb> {
        self.bounds
    }

    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FaceStyle {
    pub color: Color,
    pub pick: Option<PickId>,
}

#[derive(Debug, Clone)]
pub struct MeshInstance {
    pub mesh: Arc<ShadedMesh>,
    pub faces: Vec<FaceStyle>,
    pub placement: Option<RigidTransform>,
}

impl PartialEq for MeshInstance {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.mesh, &other.mesh)
            && self.faces == other.faces
            && self.placement == other.placement
    }
}

#[derive(Debug, Default, PartialEq)]
struct MeshPart {
    vertices: Vec<u32>,
    indices: Vec<u32>,
}

impl MeshPart {
    fn has_room(&self, max_vertices: usize, max_indices: usize) -> bool {
        self.vertices.len() + 3 <= max_vertices && self.indices.len() + 3 <= max_indices
    }
}

fn split_into_parts(mesh: &ShadedMesh, limit: u64) -> Option<Vec<MeshPart>> {
    let max_vertices = usize::try_from(limit / MESH_VERTEX_STRIDE).unwrap_or(usize::MAX);
    let max_indices = usize::try_from(limit / INDEX_BYTES / 3 * 3).unwrap_or(usize::MAX);
    if mesh.vertices.len() <= max_vertices && mesh.indices.len() <= max_indices {
        return None;
    }
    let mut local = vec![u32::MAX; mesh.vertices.len()];
    let mut parts = Vec::new();
    let mut part = MeshPart::default();
    if !part.has_room(max_vertices, max_indices) {
        return Some(parts);
    }
    for triangle in mesh.indices.as_chunks::<3>().0 {
        if triangle
            .iter()
            .any(|vertex| local.get(*vertex as usize).is_none())
        {
            continue;
        }
        if !part.has_room(max_vertices, max_indices) {
            for vertex in &part.vertices {
                if let Some(slot) = local.get_mut(*vertex as usize) {
                    *slot = u32::MAX;
                }
            }
            parts.push(std::mem::take(&mut part));
        }
        for vertex in triangle {
            let Some(slot) = local.get_mut(*vertex as usize) else {
                break;
            };
            if *slot == u32::MAX {
                *slot = u32::try_from(part.vertices.len()).unwrap_or(u32::MAX);
                part.vertices.push(*vertex);
            }
            part.indices.push(*slot);
        }
    }
    if !part.indices.is_empty() {
        parts.push(part);
    }
    Some(parts)
}

struct GpuPart {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
}

impl GpuPart {
    fn new<'a>(
        device: &wgpu::Device,
        bytes: &mut Bytes,
        vertices: impl Iterator<Item = &'a GpuVertex>,
        indices: impl Iterator<Item = u32>,
    ) -> Self {
        bytes.clear();
        for vertex in vertices {
            bytes
                .vec3(vertex.position)
                .vec3(vertex.normal)
                .u32(vertex.face);
        }
        let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mesh vertices"),
            contents: bytes.as_slice(),
            usage: wgpu::BufferUsages::VERTEX,
        });
        bytes.clear();
        let mut index_count = 0u32;
        for index in indices {
            bytes.u32(index);
            index_count = index_count.saturating_add(1);
        }
        let indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mesh indices"),
            contents: bytes.as_slice(),
            usage: wgpu::BufferUsages::INDEX,
        });
        Self {
            vertices,
            indices,
            index_count,
        }
    }
}

struct GpuMesh {
    mesh: Arc<ShadedMesh>,
    parts: Arc<[GpuPart]>,
    layout: StyleLayout,
    placement: wgpu::Buffer,
    styles: wgpu::Texture,
    written: Option<Vec<FaceStyle>>,
    placed: Option<PlacedAt>,
    corners: Option<[Point3; 8]>,
    bind_group: wgpu::BindGroup,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct PlacedAt {
    placement: Option<RigidTransform>,
    eye: Point3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StyleLayout {
    columns: u32,
    rows: u32,
    faces: u32,
}

impl StyleLayout {
    fn new(face_count: usize, largest_side: u32) -> Self {
        let side = largest_side.max(1);
        let wanted = u32::try_from(face_count).unwrap_or(u32::MAX).max(1);
        let columns = wanted.min(side);
        let rows = wanted.div_ceil(columns).min(side);
        Self {
            columns,
            rows,
            faces: columns.saturating_mul(rows).min(wanted),
        }
    }

    fn extent(self) -> wgpu::Extent3d {
        wgpu::Extent3d {
            width: self.columns,
            height: self.rows,
            depth_or_array_layers: 1,
        }
    }

    fn texels(self) -> usize {
        usize::try_from(u64::from(self.columns) * u64::from(self.rows)).unwrap_or(usize::MAX)
    }
}

fn pack_color(color: Color) -> u32 {
    color.to_array().iter().rev().fold(0, |packed, channel| {
        (packed << 8) | (channel.clamp(0.0, 1.0) * 255.0).round() as u32
    })
}

impl GpuMesh {
    fn new(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, mesh: Arc<ShadedMesh>) -> Self {
        let buffer_limit = gpu::buffer_limit(device);
        let mut bytes = Bytes::default();
        let parts = match split_into_parts(&mesh, buffer_limit) {
            None => vec![GpuPart::new(
                device,
                &mut bytes,
                mesh.vertices.iter(),
                mesh.indices.iter().copied(),
            )],
            Some(parts) => {
                log::warn!(
                    "a mesh of {} vertices is drawn in {} parts, since the graphics device holds at most {buffer_limit} bytes in a buffer",
                    mesh.vertices.len(),
                    parts.len()
                );
                parts
                    .iter()
                    .map(|part| {
                        GpuPart::new(
                            device,
                            &mut bytes,
                            part.vertices
                                .iter()
                                .filter_map(|vertex| mesh.vertices.get(*vertex as usize)),
                            part.indices.iter().copied(),
                        )
                    })
                    .collect()
            }
        };
        Self::with_parts(device, layout, mesh, parts.into())
    }

    fn sharing(&self, device: &wgpu::Device, layout: &wgpu::BindGroupLayout) -> Self {
        Self::with_parts(
            device,
            layout,
            Arc::clone(&self.mesh),
            Arc::clone(&self.parts),
        )
    }

    fn with_parts(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        mesh: Arc<ShadedMesh>,
        parts: Arc<[GpuPart]>,
    ) -> Self {
        let style_layout =
            StyleLayout::new(mesh.face_count, device.limits().max_texture_dimension_2d);
        let placement = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("mesh placement"),
            size: PLACEMENT_BYTES,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let styles = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("mesh face styles"),
            size: style_layout.extent(),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: STYLE_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let styles_view = styles.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mesh face styles"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: STYLE_BINDING,
                    resource: wgpu::BindingResource::TextureView(&styles_view),
                },
                wgpu::BindGroupEntry {
                    binding: PLACEMENT_BINDING,
                    resource: placement.as_entire_binding(),
                },
            ],
        });
        Self {
            mesh,
            parts,
            layout: style_layout,
            placement,
            styles,
            written: None,
            placed: None,
            corners: None,
            bind_group,
        }
    }

    fn write_styles(
        &mut self,
        queue: &wgpu::Queue,
        bytes: &mut Bytes,
        instance: &MeshInstance,
        eye: Point3,
    ) {
        self.write_placement(queue, bytes, instance.placement, eye);
        if self.written.as_deref() != Some(instance.faces.as_slice()) {
            self.write_face_styles(queue, bytes, instance);
        }
    }

    fn write_placement(
        &mut self,
        queue: &wgpu::Queue,
        bytes: &mut Bytes,
        placement: Option<RigidTransform>,
        eye: Point3,
    ) {
        let placed = PlacedAt { placement, eye };
        if self.placed == Some(placed) {
            return;
        }
        self.placed = Some(placed);
        self.corners = self
            .mesh
            .bounds
            .map(|bounds| placed_corners(bounds, placement));
        let placement = placement.unwrap_or(RigidTransform::IDENTITY);
        let turn = |axis: Vector3| placement.apply_vector(axis).as_vec3();
        bytes.clear();
        bytes
            .vec4(
                relative_to_eye(placement.apply_point(self.mesh.origin), eye),
                0.0,
            )
            .u32(self.layout.faces)
            .u32(self.layout.columns)
            .u32(0)
            .u32(0)
            .vec4(turn(Vector3::X), 0.0)
            .vec4(turn(Vector3::Y), 0.0)
            .vec4(turn(Vector3::Z), 0.0);
        queue.write_buffer(&self.placement, 0, bytes.as_slice());
    }

    fn write_face_styles(
        &mut self,
        queue: &wgpu::Queue,
        bytes: &mut Bytes,
        instance: &MeshInstance,
    ) {
        bytes.clear();
        for face in 0..self.layout.texels() {
            let style = instance.faces.get(face).copied().unwrap_or(UNSTYLED_FACE);
            bytes
                .u32(pack_color(style.color))
                .u32(PickId::raw(style.pick));
        }
        queue.write_texture(
            self.styles.as_image_copy(),
            bytes.as_slice(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(self.layout.columns.saturating_mul(STYLE_TEXEL_BYTES)),
                rows_per_image: Some(self.layout.rows),
            },
            self.layout.extent(),
        );
        self.written = Some(instance.faces.clone());
    }
}

pub struct MeshCache {
    layout: wgpu::BindGroupLayout,
    meshes: Vec<GpuMesh>,
    rejected: Vec<Arc<ShadedMesh>>,
    staging: Bytes,
}

impl MeshCache {
    pub fn new(device: &wgpu::Device) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("mesh face styles"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: STYLE_BINDING,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: PLACEMENT_BINDING,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        Self {
            layout,
            meshes: Vec::new(),
            rejected: Vec::new(),
            staging: Bytes::default(),
        }
    }

    pub fn layout(&self) -> &wgpu::BindGroupLayout {
        &self.layout
    }

    pub fn sibling(&self, device: &wgpu::Device) -> Self {
        Self {
            layout: self.layout.clone(),
            meshes: self
                .meshes
                .iter()
                .map(|mesh| mesh.sharing(device, &self.layout))
                .collect(),
            rejected: self.rejected.clone(),
            staging: Bytes::default(),
        }
    }

    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instances: &[MeshInstance],
        eye: Point3,
    ) -> u32 {
        let mut previous = std::mem::take(&mut self.meshes);
        let mut rejected = std::mem::take(&mut self.rejected);
        let mut kept_rejected = Vec::new();
        let mut newly_rejected = 0;
        for instance in instances
            .iter()
            .filter(|instance| !instance.mesh.is_empty())
        {
            if let Some(index) = rejected
                .iter()
                .position(|refused| Arc::ptr_eq(refused, &instance.mesh))
            {
                kept_rejected.push(rejected.swap_remove(index));
                continue;
            }
            let reused = previous
                .iter()
                .position(|cached| Arc::ptr_eq(&cached.mesh, &instance.mesh))
                .map(|index| previous.swap_remove(index));
            let (gpu, error) = gpu::scoped(device, || {
                let mut gpu = reused.unwrap_or_else(|| {
                    GpuMesh::new(device, &self.layout, Arc::clone(&instance.mesh))
                });
                gpu.write_styles(queue, &mut self.staging, instance, eye);
                gpu
            });
            match error {
                None => self.meshes.push(gpu),
                Some(error) => {
                    log::warn!(
                        "the graphics device refused a mesh of {} vertices, so it is not drawn: {error}",
                        instance.mesh.vertices.len()
                    );
                    kept_rejected.push(Arc::clone(&instance.mesh));
                    newly_rejected += 1;
                }
            }
        }
        self.rejected = kept_rejected;
        newly_rejected
    }

    pub fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        pipeline: &wgpu::RenderPipeline,
        window: &ClipWindow,
    ) {
        let mut seen = self
            .meshes
            .iter()
            .filter(|mesh| mesh.corners.is_none_or(|corners| window.sees(&corners)))
            .peekable();
        if seen.peek().is_none() {
            return;
        }
        pass.set_pipeline(pipeline);
        for mesh in seen {
            pass.set_bind_group(1, &mesh.bind_group, &[]);
            for part in mesh.parts.iter() {
                pass.set_vertex_buffer(0, part.vertices.slice(..));
                pass.set_index_buffer(part.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..part.index_count, 0, 0..1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(x: f64, y: f64, z: f64) -> MeshPoint {
        MeshPoint {
            position: Point3::new(x, y, z),
            normal: Vector3::Z * 2.0,
        }
    }

    #[test]
    fn faces_share_one_vertex_list_around_the_centre_and_bad_triangles_are_dropped() {
        let mesh = ShadedMesh::new([
            MeshFace {
                points: vec![
                    point(0.0, 0.0, 0.0),
                    point(10.0, 0.0, 0.0),
                    point(0.0, 10.0, 0.0),
                ],
                triangles: vec![[0, 1, 2], [0, 1, 3]],
            },
            MeshFace {
                points: vec![
                    point(0.0, 0.0, 4.0),
                    point(10.0, 0.0, 4.0),
                    point(0.0, 10.0, 4.0),
                ],
                triangles: vec![[2, 1, 0]],
            },
        ]);

        assert_eq!(mesh.face_count(), 2);
        assert_eq!(mesh.indices, vec![0, 1, 2, 5, 4, 3]);
        assert_eq!(mesh.origin, Point3::new(5.0, 5.0, 2.0));
        assert_eq!(mesh.vertices[4].position, Vec3::new(5.0, -5.0, 2.0));
        assert_eq!(mesh.vertices[4].face, 1);
        assert_eq!(mesh.vertices[4].normal, Vec3::Z);
        assert!(!mesh.is_empty());
        assert!(ShadedMesh::new([]).is_empty());
    }

    #[test]
    fn face_styles_fill_rows_of_a_texture_no_wider_than_the_device_allows() {
        let one_row = StyleLayout::new(6, 8192);
        let wrapped = StyleLayout::new(3000, 256);
        let beyond = StyleLayout::new(100_000, 64);

        assert_eq!(
            one_row,
            StyleLayout {
                columns: 6,
                rows: 1,
                faces: 6,
            }
        );
        assert_eq!(
            wrapped,
            StyleLayout {
                columns: 256,
                rows: 12,
                faces: 3000,
            }
        );
        assert_eq!(wrapped.texels(), 3072);
        assert_eq!(beyond.faces, 64 * 64);
        assert_eq!(StyleLayout::new(0, 0).faces, 1);
    }

    #[test]
    fn colours_pack_red_into_the_lowest_byte() {
        assert_eq!(pack_color(Color::from_rgba8(1, 2, 3, 4)), 0x0403_0201);
        assert_eq!(
            pack_color(Color::from_rgb8(255, 0, 0).with_alpha(2.0)),
            0xff00_00ff
        );
    }

    fn strip(quads: u32) -> ShadedMesh {
        let points = (0..=quads)
            .flat_map(|column| {
                let x = f64::from(column);
                [point(x, 0.0, 0.0), point(x, 1.0, 0.0)]
            })
            .collect();
        let triangles = (0..quads)
            .flat_map(|quad| {
                let first = quad * 2;
                [[first, first + 2, first + 3], [first, first + 3, first + 1]]
            })
            .collect();
        ShadedMesh::new([MeshFace { points, triangles }])
    }

    fn triangles_in_space(mesh: &ShadedMesh, parts: &[MeshPart]) -> Vec<[Vec3; 3]> {
        parts
            .iter()
            .flat_map(|part| {
                part.indices.as_chunks::<3>().0.iter().map(|triangle| {
                    triangle
                        .map(|local| mesh.vertices[part.vertices[local as usize] as usize].position)
                })
            })
            .collect()
    }

    #[test]
    fn a_mesh_larger_than_a_buffer_is_split_into_parts_that_each_fit() {
        let mesh = strip(100);
        let limit = 40 * MESH_VERTEX_STRIDE;

        let parts = split_into_parts(&mesh, limit).unwrap();
        let expected: Vec<[Vec3; 3]> = mesh
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .map(|triangle| triangle.map(|index| mesh.vertices[index as usize].position))
            .collect();

        assert!(parts.len() >= 6, "{}", parts.len());
        for part in &parts {
            assert!(part.vertices.len() as u64 * MESH_VERTEX_STRIDE <= limit);
            assert!(part.indices.len() as u64 * INDEX_BYTES <= limit);
            assert!(
                part.indices
                    .iter()
                    .all(|local| (*local as usize) < part.vertices.len())
            );
        }
        assert_eq!(triangles_in_space(&mesh, &parts), expected);
        assert_eq!(split_into_parts(&mesh, 1 << 20), None);
        assert_eq!(split_into_parts(&mesh, 8), Some(Vec::new()));
    }
}
