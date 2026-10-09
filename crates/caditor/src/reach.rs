use std::sync::{Arc, OnceLock, Weak};

use caditor_geometry::{Point2, Point3, Vector2, Vector3};
use caditor_render::ShadedMesh;

const MOST_CELLS_ACROSS: usize = 512;
const SLACK_PER_SIZE: f64 = 1e-6;
const SMALLEST_SLACK: f64 = 1e-9;
const NUDGES_PER_SLACK: f64 = 4.0;
pub const FACING_SLACK: f64 = 1e-6;

pub struct Occluders {
    pub generation: u64,
    pub reach: Vector3,
    meshes: Vec<Weak<ShadedMesh>>,
    pub triangles: usize,
    grid: OnceLock<Grid>,
}

impl Occluders {
    pub fn new(generation: u64, reach: Vector3, meshes: &[Arc<ShadedMesh>]) -> Self {
        Self {
            generation,
            reach,
            meshes: meshes.iter().map(Arc::downgrade).collect(),
            triangles: meshes.iter().map(|mesh| mesh.triangle_count()).sum(),
            grid: OnceLock::new(),
        }
    }

    pub fn is_of(&self, reach: Vector3, meshes: &[Arc<ShadedMesh>]) -> bool {
        self.reach == reach
            && self.meshes.len() == meshes.len()
            && self
                .meshes
                .iter()
                .zip(meshes)
                .all(|(kept, mesh)| kept.ptr_eq(&Arc::downgrade(mesh)))
    }

    pub fn grid(&self) -> &Grid {
        self.grid.get_or_init(|| {
            let meshes: Vec<Arc<ShadedMesh>> =
                self.meshes.iter().filter_map(Weak::upgrade).collect();
            Grid::of(self.reach, &meshes)
        })
    }
}

#[derive(Debug, Clone, Copy)]
struct Frame {
    across: Vector3,
    along: Vector3,
    reach: Vector3,
}

impl Frame {
    fn new(reach: Vector3) -> Self {
        let (across, along) = reach.any_orthonormal_pair();
        Self {
            across,
            along,
            reach,
        }
    }

    fn project(self, point: Point3) -> (Point2, f64) {
        (
            Point2::new(point.dot(self.across), point.dot(self.along)),
            point.dot(self.reach),
        )
    }
}

#[derive(Debug, Clone, Copy)]
struct Shadow {
    corners: [Point2; 3],
    heights: [f64; 3],
    lowest: Point2,
    highest: Point2,
    area: f64,
}

impl Shadow {
    fn of(corners: [Point2; 3], heights: [f64; 3], slack: f64) -> Option<Self> {
        let [a, b, c] = corners;
        let area = (b - a).perp_dot(c - a);
        let longest = [b - a, c - b, a - c]
            .into_iter()
            .map(Vector2::length)
            .fold(0.0, f64::max);
        if area.abs() <= slack * longest {
            return None;
        }
        Some(Self {
            corners,
            heights,
            lowest: a.min(b).min(c),
            highest: a.max(b).max(c),
            area,
        })
    }

    fn height_over(&self, point: Point2, slack: f64) -> Option<f64> {
        if point.cmplt(self.lowest).any() || point.cmpgt(self.highest).any() {
            return None;
        }
        let [a, b, c] = self.corners;
        let sign = self.area.signum();
        let mut weights = [0.0; 3];
        for (weight, (from, to)) in weights.iter_mut().zip([(b, c), (c, a), (a, b)]) {
            let edge = to - from;
            let signed = edge.perp_dot(point - from) * sign;
            if signed <= slack * edge.length() {
                return None;
            }
            *weight = signed;
        }
        let total: f64 = weights.iter().sum();
        let height = weights
            .iter()
            .zip(self.heights)
            .map(|(weight, height)| weight * height)
            .sum::<f64>()
            / total;
        Some(height)
    }
}

pub struct Grid {
    frame: Frame,
    shadows: Vec<Shadow>,
    cells: Vec<Vec<u32>>,
    corner: Point2,
    cell: f64,
    columns: usize,
    rows: usize,
    slack: f64,
}

impl Grid {
    pub fn of(reach: Vector3, meshes: &[Arc<ShadedMesh>]) -> Self {
        let frame = Frame::new(reach);
        let size = meshes
            .iter()
            .filter_map(|mesh| mesh.bounds())
            .reduce(|all, bounds| all.union(bounds))
            .map_or(0.0, |bounds| bounds.diagonal());
        let slack = (size * SLACK_PER_SIZE).max(SMALLEST_SLACK);
        let shadows: Vec<Shadow> = meshes
            .iter()
            .flat_map(|mesh| mesh.face_triangles())
            .filter_map(|(_, corners)| {
                let projected = corners.map(|corner| frame.project(corner));
                Shadow::of(
                    projected.map(|(point, _)| point),
                    projected.map(|(_, height)| height),
                    slack,
                )
            })
            .collect();
        let lowest = shadows
            .iter()
            .map(|shadow| shadow.lowest)
            .reduce(Point2::min)
            .unwrap_or(Point2::ZERO);
        let highest = shadows
            .iter()
            .map(|shadow| shadow.highest)
            .reduce(Point2::max)
            .unwrap_or(Point2::ZERO);
        let extent = highest - lowest;
        let across = ((shadows.len() as f64).sqrt().ceil() as usize).clamp(1, MOST_CELLS_ACROSS);
        let cell = (extent.max_element() / across as f64).max(slack);
        let count = |length: f64| ((length / cell).floor() as usize + 1).min(MOST_CELLS_ACROSS);
        let (columns, rows) = (count(extent.x), count(extent.y));
        let mut grid = Self {
            frame,
            shadows: Vec::new(),
            cells: vec![Vec::new(); columns * rows],
            corner: lowest,
            cell,
            columns,
            rows,
            slack,
        };
        for (index, shadow) in shadows.iter().enumerate() {
            let Ok(index) = u32::try_from(index) else {
                break;
            };
            let (first_column, first_row) = grid.cell_of(shadow.lowest);
            let (last_column, last_row) = grid.cell_of(shadow.highest);
            for row in first_row..=last_row {
                for column in first_column..=last_column {
                    if let Some(cell) = grid.cells.get_mut(row * columns + column) {
                        cell.push(index);
                    }
                }
            }
        }
        grid.shadows = shadows;
        grid
    }

    fn cell_of(&self, point: Point2) -> (usize, usize) {
        let at = (point - self.corner) / self.cell;
        let clamp = |value: f64, count: usize| (value.max(0.0) as usize).min(count - 1);
        (clamp(at.x, self.columns), clamp(at.y, self.rows))
    }

    pub fn blocks(&self, point: Point3, normal: Vector3) -> bool {
        let nudged = point + normal * (self.slack * NUDGES_PER_SLACK);
        let (spot, height) = self.frame.project(nudged);
        let (column, row) = self.cell_of(spot);
        let Some(cell) = self.cells.get(row * self.columns + column) else {
            return false;
        };
        cell.iter().any(|index| {
            self.shadows
                .get(*index as usize)
                .and_then(|shadow| shadow.height_over(spot, self.slack))
                .is_some_and(|above| above > height + self.slack)
        })
    }
}

#[cfg(test)]
mod tests {
    use caditor_render::{MeshFace, MeshPoint};

    use super::*;

    fn square(z: f64, from: f64, to: f64, up: bool) -> MeshFace {
        let normal = if up { Vector3::Z } else { -Vector3::Z };
        let at = |x: f64, y: f64| MeshPoint {
            position: Point3::new(x, y, z),
            normal,
        };
        MeshFace {
            points: vec![at(from, from), at(to, from), at(to, to), at(from, to)],
            triangles: vec![[0, 1, 2], [0, 2, 3]],
        }
    }

    #[test]
    fn a_point_is_blocked_only_by_a_face_above_it_along_the_reach() {
        let roof = Arc::new(ShadedMesh::new([square(5.0, 0.0, 2.0, false)]));
        let grid = Grid::of(Vector3::Z, &[roof]);

        assert!(grid.blocks(Point3::new(1.5, 0.5, 0.0), Vector3::Z));
        assert!(!grid.blocks(Point3::new(1.5, 0.5, 6.0), Vector3::Z));
        assert!(!grid.blocks(Point3::new(3.0, 1.0, 0.0), Vector3::Z));
        assert!(!grid.blocks(Point3::new(1.5, 0.5, 5.0), Vector3::Z));
    }

    #[test]
    fn a_face_square_to_the_reach_casts_no_shadow() {
        let at = |x: f64, z: f64| MeshPoint {
            position: Point3::new(x, 0.0, z),
            normal: Vector3::Y,
        };
        let wall = Arc::new(ShadedMesh::new([MeshFace {
            points: vec![at(0.0, 0.0), at(2.0, 0.0), at(2.0, 2.0), at(0.0, 2.0)],
            triangles: vec![[0, 1, 2], [0, 2, 3]],
        }]));
        let grid = Grid::of(Vector3::Z, &[wall]);

        assert!(!grid.blocks(Point3::new(1.0, 0.0, -1.0), Vector3::Y));
        assert!(grid.shadows.is_empty());
    }
}
