use std::sync::Arc;

use caditor_geometry::{Aabb, Point3, Vector3};
use glam::Vec3;
use wgpu::util::DeviceExt;

use crate::{
    gpu::{self, Bytes},
    scene::{Color, PickId},
    viewport::relative_to_eye,
};

pub const MESH_VERTEX_STRIDE: u64 = 28;
const INDEX_BYTES: u64 = 4;
const STYLE_BINDING: u32 = 1;
const STYLE_HEADER_BYTES: u64 = 16;
const FACE_STYLE_BYTES: u64 = 32;
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
}

impl PartialEq for MeshInstance {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.mesh, &other.mesh) && self.faces == other.faces
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
    parts: Vec<GpuPart>,
    styled_faces: usize,
    styles: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

impl GpuMesh {
    fn new(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, mesh: Arc<ShadedMesh>) -> Self {
        let limits = device.limits();
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
        let style_limit = buffer_limit.min(limits.max_storage_buffer_binding_size);
        let styled_faces = usize::try_from(
            (style_limit.saturating_sub(STYLE_HEADER_BYTES) / FACE_STYLE_BYTES).max(1),
        )
        .unwrap_or(usize::MAX)
        .min(mesh.face_count.max(1));
        let styles = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("mesh face styles"),
            size: STYLE_HEADER_BYTES
                .saturating_add((styled_faces as u64).saturating_mul(FACE_STYLE_BYTES)),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mesh face styles"),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: STYLE_BINDING,
                resource: styles.as_entire_binding(),
            }],
        });
        Self {
            mesh,
            parts,
            styled_faces,
            styles,
            bind_group,
        }
    }

    fn write_styles(
        &self,
        queue: &wgpu::Queue,
        bytes: &mut Bytes,
        instance: &MeshInstance,
        eye: Point3,
    ) {
        bytes.clear();
        bytes.vec4(relative_to_eye(self.mesh.origin, eye), 0.0);
        for face in 0..self.styled_faces {
            let style = instance.faces.get(face).copied().unwrap_or(UNSTYLED_FACE);
            bytes
                .floats(&style.color.to_array())
                .u32(PickId::raw(style.pick))
                .u32(0)
                .u32(0)
                .u32(0);
        }
        queue.write_buffer(&self.styles, 0, bytes.as_slice());
    }
}

pub struct MeshCache {
    layout: wgpu::BindGroupLayout,
    meshes: Vec<GpuMesh>,
    staging: Bytes,
}

impl MeshCache {
    pub fn new(device: &wgpu::Device) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("mesh face styles"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: STYLE_BINDING,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        Self {
            layout,
            meshes: Vec::new(),
            staging: Bytes::default(),
        }
    }

    pub fn layout(&self) -> &wgpu::BindGroupLayout {
        &self.layout
    }

    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instances: &[MeshInstance],
        eye: Point3,
    ) {
        let mut previous = std::mem::take(&mut self.meshes);
        for instance in instances
            .iter()
            .filter(|instance| !instance.mesh.is_empty())
        {
            let reused = previous
                .iter()
                .position(|cached| Arc::ptr_eq(&cached.mesh, &instance.mesh))
                .map(|index| previous.swap_remove(index));
            let gpu = reused
                .unwrap_or_else(|| GpuMesh::new(device, &self.layout, Arc::clone(&instance.mesh)));
            gpu.write_styles(queue, &mut self.staging, instance, eye);
            self.meshes.push(gpu);
        }
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, pipeline: &wgpu::RenderPipeline) {
        if self.meshes.is_empty() {
            return;
        }
        pass.set_pipeline(pipeline);
        for mesh in &self.meshes {
            pass.set_bind_group(1, &mesh.bind_group, &[]);
            for part in &mesh.parts {
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
