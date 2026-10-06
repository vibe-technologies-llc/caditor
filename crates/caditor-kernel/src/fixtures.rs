use std::{collections::BTreeMap, f64::consts::FRAC_PI_2};

use caditor_geometry::{Plane, Point3, RigidTransform, Vector3};

use crate::{
    bspline::BSpline,
    curve::{Circle, Curve},
    interval::Interval,
    sense::Sense,
    surface::{BSplineSurface, Cone, Cylinder, Extrusion, PlaneSurface, Sphere, Surface, Torus},
    topology::{EdgeId, FaceId, ShellId, Solid, SolidBuilder, VertexId},
};

pub(crate) type Coedges = Vec<(EdgeId, Sense)>;

pub(crate) struct Fixture {
    pub builder: SolidBuilder,
    shell: ShellId,
    lines: BTreeMap<(VertexId, VertexId), EdgeId>,
}

impl Fixture {
    pub fn new() -> Self {
        let mut builder = SolidBuilder::new();
        let shell = builder.shell().unwrap();
        Self {
            builder,
            shell,
            lines: BTreeMap::new(),
        }
    }

    pub fn next_shell(&mut self) {
        self.shell = self.builder.shell().unwrap();
    }

    pub fn vertex(&mut self, point: Point3) -> VertexId {
        self.builder.vertex(point).unwrap()
    }

    pub fn line(&mut self, from: VertexId, to: VertexId) -> (EdgeId, Sense) {
        if let Some(edge) = self.lines.get(&(to, from)) {
            return (*edge, Sense::Reversed);
        }
        let edge = self.builder.line_edge(from, to).unwrap();
        self.lines.insert((from, to), edge);
        (edge, Sense::Same)
    }

    pub fn edge(
        &mut self,
        curve: impl Into<Curve>,
        interval: Interval,
        from: VertexId,
        to: VertexId,
    ) -> EdgeId {
        self.builder.edge(curve.into(), interval, from, to).unwrap()
    }

    pub fn face(&mut self, surface: impl Into<Surface>, sense: Sense, loops: &[Coedges]) -> FaceId {
        let face = self
            .builder
            .face(self.shell, surface.into(), sense)
            .unwrap();
        for coedges in loops {
            self.builder.add_loop(face, coedges).unwrap();
        }
        face
    }

    pub fn point(&self, vertex: VertexId) -> Point3 {
        self.builder.vertex_point(vertex).unwrap()
    }

    pub fn polygon_plane(&self, corners: &[VertexId]) -> Plane {
        let points: Vec<Point3> = corners.iter().map(|corner| self.point(*corner)).collect();
        let normal = points
            .iter()
            .zip(points.iter().cycle().skip(1))
            .fold(Vector3::ZERO, |sum, (a, b)| sum + a.cross(*b));
        Plane::with_x_axis(points[0], normal, points[1] - points[0]).unwrap()
    }

    pub fn polygon_loop(&mut self, corners: &[VertexId]) -> Coedges {
        (0..corners.len())
            .map(|index| self.line(corners[index], corners[(index + 1) % corners.len()]))
            .collect()
    }

    pub fn polygon(&mut self, corners: &[VertexId], holes: &[Coedges]) -> FaceId {
        let plane = self.polygon_plane(corners);
        let mut loops = vec![self.polygon_loop(corners)];
        loops.extend(holes.iter().cloned());
        self.face(PlaneSurface::new(plane).unwrap(), Sense::Same, &loops)
    }

    pub fn build(self) -> Solid {
        match self.builder.build() {
            Ok(solid) => solid,
            Err(error) => panic!("fixture does not validate: {error}"),
        }
    }

    pub fn build_unchecked(self) -> Solid {
        self.builder.build_unchecked()
    }
}

pub(crate) const CUBOID_FACES: [[usize; 4]; 6] = [
    [0, 2, 3, 1],
    [4, 5, 7, 6],
    [0, 1, 5, 4],
    [2, 6, 7, 3],
    [0, 4, 6, 2],
    [1, 3, 7, 5],
];

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Tweak {
    pub flipped: Option<usize>,
    pub missing: Option<usize>,
    pub shifted: Option<(usize, f64)>,
    pub scrambled: Option<usize>,
    pub inverted: bool,
}

pub(crate) fn cuboid_vertices(fixture: &mut Fixture, min: Point3, max: Point3) -> Vec<VertexId> {
    (0..8)
        .map(|index| {
            let pick = |bit: usize, low: f64, high: f64| if index & bit == 0 { low } else { high };
            fixture.vertex(Point3::new(
                pick(1, min.x, max.x),
                pick(2, min.y, max.y),
                pick(4, min.z, max.z),
            ))
        })
        .collect()
}

pub(crate) fn add_cuboid(fixture: &mut Fixture, min: Point3, max: Point3, tweak: Tweak) {
    let vertices = cuboid_vertices(fixture, min, max);
    for (index, face) in CUBOID_FACES.iter().enumerate() {
        if tweak.missing == Some(index) {
            continue;
        }
        let mut corners: Vec<VertexId> = face.iter().map(|corner| vertices[*corner]).collect();
        if tweak.inverted {
            corners.reverse();
        }
        let mut plane = fixture.polygon_plane(&corners);
        if let Some((_, offset)) = tweak.shifted.filter(|(shifted, _)| *shifted == index) {
            plane = Plane::from_frame(
                plane.origin() + plane.normal() * offset,
                plane.normal(),
                plane.x_axis(),
            )
            .unwrap();
        }
        let mut coedges = fixture.polygon_loop(&corners);
        if tweak.scrambled == Some(index) {
            coedges.swap(1, 2);
        }
        let sense = if tweak.flipped == Some(index) {
            Sense::Reversed
        } else {
            Sense::Same
        };
        let face = fixture
            .builder
            .face(
                fixture.shell,
                PlaneSurface::new(plane).unwrap().into(),
                sense,
            )
            .unwrap();
        fixture.builder.add_loop(face, &coedges).unwrap();
    }
}

pub(crate) fn cuboid(size: Vector3) -> Solid {
    let mut fixture = Fixture::new();
    add_cuboid(&mut fixture, Point3::ZERO, size, Tweak::default());
    fixture.build()
}

pub(crate) fn tweaked_cuboid(tweak: Tweak) -> Solid {
    let mut fixture = Fixture::new();
    add_cuboid(
        &mut fixture,
        Point3::ZERO,
        Point3::new(4.0, 3.0, 2.0),
        tweak,
    );
    fixture.build_unchecked()
}

pub(crate) fn spline_topped_block(side: f64, height: f64, bulge: f64) -> Solid {
    let mut control_points = Vec::with_capacity(16);
    for row in 0..4 {
        for column in 0..4 {
            let raised = (1..=2).contains(&row) && (1..=2).contains(&column);
            control_points.push(Point3::new(
                side * column as f64 / 3.0,
                side * row as f64 / 3.0,
                if raised { height + bulge } else { height },
            ));
        }
    }
    let knots = vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0];
    let surface = BSplineSurface::new(3, 3, knots.clone(), knots, 4, control_points, None).unwrap();
    block_topped_by(side, height, surface)
}

pub(crate) fn bumped_sheet(side: f64, height: f64, nodes: usize, bump: f64) -> BSplineSurface {
    let middle = nodes / 2;
    let control_points = (0..nodes)
        .flat_map(|row| {
            (0..nodes).map(move |column| {
                let raised = row == middle && column == middle;
                Point3::new(
                    side * column as f64 / (nodes - 1) as f64,
                    side * row as f64 / (nodes - 1) as f64,
                    if raised { height + bump } else { height },
                )
            })
        })
        .collect();
    let knots: Vec<f64> = [0.0; 3]
        .into_iter()
        .chain((0..=nodes - 3).map(|knot| knot as f64 / (nodes - 3) as f64))
        .chain([1.0; 3])
        .collect();
    BSplineSurface::new(3, 3, knots.clone(), knots, nodes, control_points, None).unwrap()
}

pub(crate) fn bumped_block(side: f64, height: f64, nodes: usize, bump: f64) -> Solid {
    block_topped_by(side, height, bumped_sheet(side, height, nodes, bump))
}

fn block_topped_by(side: f64, height: f64, top: BSplineSurface) -> Solid {
    let mut fixture = Fixture::new();
    let vertices = cuboid_vertices(&mut fixture, Point3::ZERO, Point3::new(side, side, height));
    for (index, face) in CUBOID_FACES.iter().enumerate() {
        let corners: Vec<VertexId> = face.iter().map(|corner| vertices[*corner]).collect();
        if index == 1 {
            let coedges = fixture.polygon_loop(&corners);
            fixture.face(top.clone(), Sense::Same, &[coedges]);
        } else {
            fixture.polygon(&corners, &[]);
        }
    }
    fixture.build()
}

pub(crate) fn hollow_cuboid(outer: f64, inner: f64) -> Solid {
    let mut fixture = Fixture::new();
    add_cuboid(
        &mut fixture,
        Point3::ZERO,
        Point3::splat(outer),
        Tweak::default(),
    );
    fixture.next_shell();
    let gap = 0.5 * (outer - inner);
    add_cuboid(
        &mut fixture,
        Point3::splat(gap),
        Point3::splat(gap + inner),
        Tweak {
            inverted: true,
            ..Tweak::default()
        },
    );
    fixture.build()
}

fn horizontal(height: f64) -> Plane {
    Plane::from_frame(Point3::new(0.0, 0.0, height), Vector3::Z, Vector3::X).unwrap()
}

fn centered(center: Point3) -> Plane {
    Plane::from_frame(center, Vector3::Z, Vector3::X).unwrap()
}

pub(crate) fn cylinder(radius: f64, height: f64) -> Solid {
    let mut fixture = Fixture::new();
    let bottom_vertex = fixture.vertex(Point3::new(radius, 0.0, 0.0));
    let top_vertex = fixture.vertex(Point3::new(radius, 0.0, height));
    let bottom = fixture.edge(
        Circle::new(horizontal(0.0), radius).unwrap(),
        Interval::FULL_TURN,
        bottom_vertex,
        bottom_vertex,
    );
    let top = fixture.edge(
        Circle::new(horizontal(height), radius).unwrap(),
        Interval::FULL_TURN,
        top_vertex,
        top_vertex,
    );
    let (seam, _) = fixture.line(bottom_vertex, top_vertex);
    fixture.face(
        Cylinder::new(horizontal(0.0), radius).unwrap(),
        Sense::Same,
        &[vec![
            (bottom, Sense::Same),
            (seam, Sense::Same),
            (top, Sense::Reversed),
            (seam, Sense::Reversed),
        ]],
    );
    fixture.face(
        PlaneSurface::new(horizontal(height)).unwrap(),
        Sense::Same,
        &[vec![(top, Sense::Same)]],
    );
    fixture.face(
        PlaneSurface::new(horizontal(0.0).flipped()).unwrap(),
        Sense::Same,
        &[vec![(bottom, Sense::Reversed)]],
    );
    fixture.build()
}

pub(crate) fn holed_block(side: f64, height: f64, radius: f64) -> Solid {
    let mut fixture = Fixture::new();
    let corners = cuboid_vertices(&mut fixture, Point3::ZERO, Point3::new(side, side, height));
    let center = Point3::new(0.5 * side, 0.5 * side, 0.0);
    let low = fixture.vertex(center + Vector3::new(radius, 0.0, 0.0));
    let high = fixture.vertex(center + Vector3::new(radius, 0.0, height));
    let bottom = fixture.edge(
        Circle::new(centered(center), radius).unwrap(),
        Interval::FULL_TURN,
        low,
        low,
    );
    let top_center = center + Vector3::new(0.0, 0.0, height);
    let top = fixture.edge(
        Circle::new(centered(top_center), radius).unwrap(),
        Interval::FULL_TURN,
        high,
        high,
    );
    let (seam, _) = fixture.line(low, high);
    for (index, face) in CUBOID_FACES.iter().enumerate() {
        let cycle: Vec<VertexId> = face.iter().map(|corner| corners[*corner]).collect();
        let holes = match index {
            0 => vec![vec![(bottom, Sense::Same)]],
            1 => vec![vec![(top, Sense::Reversed)]],
            _ => Vec::new(),
        };
        fixture.polygon(&cycle, &holes);
    }
    fixture.face(
        Cylinder::new(centered(center), radius).unwrap(),
        Sense::Reversed,
        &[vec![
            (seam, Sense::Same),
            (top, Sense::Same),
            (seam, Sense::Reversed),
            (bottom, Sense::Reversed),
        ]],
    );
    fixture.build()
}

pub(crate) fn sphere(radius: f64) -> Solid {
    let mut fixture = Fixture::new();
    let south = fixture.vertex(Point3::new(0.0, 0.0, -radius));
    let north = fixture.vertex(Point3::new(0.0, 0.0, radius));
    let meridian = Plane::from_frame(Point3::ZERO, Vector3::NEG_Y, Vector3::X).unwrap();
    let seam = fixture.edge(
        Circle::new(meridian, radius).unwrap(),
        Interval::new(-FRAC_PI_2, FRAC_PI_2).unwrap(),
        south,
        north,
    );
    fixture.face(
        Sphere::new(Plane::XY, radius).unwrap(),
        Sense::Same,
        &[vec![(seam, Sense::Same), (seam, Sense::Reversed)]],
    );
    fixture.build()
}

pub(crate) fn torus(major: f64, minor: f64) -> Solid {
    let mut fixture = Fixture::new();
    let corner = fixture.vertex(Point3::new(major + minor, 0.0, 0.0));
    let around = fixture.edge(
        Circle::new(Plane::XY, major + minor).unwrap(),
        Interval::FULL_TURN,
        corner,
        corner,
    );
    let tube = Plane::from_frame(Point3::new(major, 0.0, 0.0), Vector3::NEG_Y, Vector3::X).unwrap();
    let across = fixture.edge(
        Circle::new(tube, minor).unwrap(),
        Interval::FULL_TURN,
        corner,
        corner,
    );
    fixture.face(
        Torus::new(Plane::XY, major, minor).unwrap(),
        Sense::Same,
        &[vec![
            (around, Sense::Same),
            (across, Sense::Same),
            (around, Sense::Reversed),
            (across, Sense::Reversed),
        ]],
    );
    fixture.build()
}

pub(crate) fn frustum(bottom_radius: f64, top_radius: f64, height: f64) -> Solid {
    let mut fixture = Fixture::new();
    let bottom_vertex = fixture.vertex(Point3::new(bottom_radius, 0.0, 0.0));
    let top_vertex = fixture.vertex(Point3::new(top_radius, 0.0, height));
    let bottom = fixture.edge(
        Circle::new(horizontal(0.0), bottom_radius).unwrap(),
        Interval::FULL_TURN,
        bottom_vertex,
        bottom_vertex,
    );
    let top = fixture.edge(
        Circle::new(horizontal(height), top_radius).unwrap(),
        Interval::FULL_TURN,
        top_vertex,
        top_vertex,
    );
    let (seam, _) = fixture.line(bottom_vertex, top_vertex);
    let half_angle = (top_radius - bottom_radius).atan2(height);
    fixture.face(
        Cone::new(horizontal(0.0), bottom_radius, half_angle).unwrap(),
        Sense::Same,
        &[vec![
            (bottom, Sense::Same),
            (seam, Sense::Same),
            (top, Sense::Reversed),
            (seam, Sense::Reversed),
        ]],
    );
    fixture.face(
        PlaneSurface::new(horizontal(height)).unwrap(),
        Sense::Same,
        &[vec![(top, Sense::Same)]],
    );
    fixture.face(
        PlaneSurface::new(horizontal(0.0).flipped()).unwrap(),
        Sense::Same,
        &[vec![(bottom, Sense::Reversed)]],
    );
    fixture.build()
}

pub(crate) fn cone(radius: f64, height: f64) -> Solid {
    let mut fixture = Fixture::new();
    let rim = fixture.vertex(Point3::new(radius, 0.0, 0.0));
    let apex = fixture.vertex(Point3::new(0.0, 0.0, height));
    let base = fixture.edge(
        Circle::new(horizontal(0.0), radius).unwrap(),
        Interval::FULL_TURN,
        rim,
        rim,
    );
    let (seam, _) = fixture.line(rim, apex);
    fixture.face(
        Cone::new(horizontal(0.0), radius, (-radius).atan2(height)).unwrap(),
        Sense::Same,
        &[vec![
            (base, Sense::Same),
            (seam, Sense::Same),
            (seam, Sense::Reversed),
        ]],
    );
    fixture.face(
        PlaneSurface::new(horizontal(0.0).flipped()).unwrap(),
        Sense::Same,
        &[vec![(base, Sense::Reversed)]],
    );
    fixture.build()
}

pub(crate) fn spline_profile() -> Curve {
    BSpline::clamped_uniform(
        3,
        vec![
            Point3::new(20.0, 0.0, 0.0),
            Point3::new(22.0, 12.0, 0.0),
            Point3::new(10.0, 18.0, 0.0),
            Point3::new(-2.0, 12.0, 0.0),
            Point3::new(0.0, 0.0, 0.0),
        ],
    )
    .unwrap()
    .into()
}

pub(crate) fn extruded_spline(height: f64) -> Solid {
    let mut fixture = Fixture::new();
    let profile = spline_profile();
    let lifted = profile
        .transformed(&RigidTransform::translation(Vector3::new(0.0, 0.0, height)).unwrap())
        .unwrap();
    let a_bottom = fixture.vertex(Point3::new(20.0, 0.0, 0.0));
    let b_bottom = fixture.vertex(Point3::ZERO);
    let a_top = fixture.vertex(Point3::new(20.0, 0.0, height));
    let b_top = fixture.vertex(Point3::new(0.0, 0.0, height));
    let bottom = fixture.edge(profile.clone(), Interval::UNIT, a_bottom, b_bottom);
    let top = fixture.edge(lifted, Interval::UNIT, a_top, b_top);
    fixture.polygon(&[b_bottom, a_bottom, a_top, b_top], &[]);
    let rise = fixture.line(b_bottom, b_top);
    let fall = fixture.line(a_top, a_bottom);
    fixture.face(
        Extrusion::new(profile, Vector3::Z).unwrap(),
        Sense::Same,
        &[vec![
            (bottom, Sense::Same),
            rise,
            (top, Sense::Reversed),
            fall,
        ]],
    );
    let floor = fixture.line(a_bottom, b_bottom);
    fixture.face(
        PlaneSurface::new(horizontal(0.0).flipped()).unwrap(),
        Sense::Same,
        &[vec![(bottom, Sense::Reversed), floor]],
    );
    let ceiling = fixture.line(b_top, a_top);
    fixture.face(
        PlaneSurface::new(horizontal(height)).unwrap(),
        Sense::Same,
        &[vec![ceiling, (top, Sense::Same)]],
    );
    fixture.build()
}

pub(crate) fn every_solid() -> Vec<(&'static str, Solid)> {
    vec![
        ("cuboid", cuboid(Vector3::new(4.0, 3.0, 2.0))),
        ("hollow cuboid", hollow_cuboid(10.0, 4.0)),
        ("cylinder", cylinder(3.0, 5.0)),
        ("holed block", holed_block(10.0, 4.0, 2.5)),
        ("sphere", sphere(4.0)),
        ("torus", torus(6.0, 2.0)),
        ("frustum", frustum(4.0, 2.0, 5.0)),
        ("cone", cone(3.0, 4.0)),
        ("extruded spline", extruded_spline(5.0)),
    ]
}
