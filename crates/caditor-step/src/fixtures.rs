use caditor_geometry::{Plane, Point2, Point3, Vector2, Vector3};
use caditor_kernel::{
    AngularExtent, Axis2, BlendShape, BooleanOperation, LinearExtent, Mesh, Profile, ProfileCurve,
    SamplingTolerance, Selection, Solid, blend, boolean, extrude, revolve,
};

fn polygon(points: &[(f64, f64)], first_entity: u64) -> Vec<ProfileCurve> {
    (0..points.len())
        .map(|index| {
            let (a, b) = (points[index], points[(index + 1) % points.len()]);
            ProfileCurve::line(
                first_entity + index as u64,
                Point2::new(a.0, a.1),
                Point2::new(b.0, b.1),
            )
        })
        .collect()
}

pub fn swept(plane: Plane, curves: &[ProfileCurve], height: f64) -> Solid {
    let profile = Profile::new(curves).unwrap();
    let regions = profile.select(&Selection::EvenDepth).unwrap();
    extrude(&plane, &regions, LinearExtent::one_side(height).unwrap(), 1).unwrap()
}

pub fn plate_with_hole() -> Solid {
    let mut curves = polygon(&[(0.0, 0.0), (40.0, 0.0), (40.0, 30.0), (0.0, 30.0)], 1);
    curves.push(ProfileCurve::circle(9, Point2::new(12.0, 15.0), 5.0));
    swept(Plane::XY, &curves, 10.0)
}

pub fn turned() -> Solid {
    let curves = vec![
        ProfileCurve::line(1, Point2::new(0.0, 0.0), Point2::new(20.0, 0.0)),
        ProfileCurve::line(2, Point2::new(20.0, 0.0), Point2::new(12.0, 15.0)),
        ProfileCurve::arc(
            3,
            Point2::new(12.0, 20.0),
            Point2::new(12.0, 15.0),
            Point2::new(12.0, 25.0),
        ),
        ProfileCurve::line(4, Point2::new(12.0, 25.0), Point2::new(0.0, 25.0)),
        ProfileCurve::line(5, Point2::new(0.0, 25.0), Point2::new(0.0, 0.0)),
    ];
    let profile = Profile::new(&curves).unwrap();
    let regions = profile.select(&Selection::EvenDepth).unwrap();
    let axis = Axis2::new(Point2::ZERO, Vector2::Y).unwrap();
    revolve(&Plane::XZ, &regions, axis, AngularExtent::full(), 1).unwrap()
}

pub fn hollow_ring() -> Solid {
    let mut curves = polygon(&[(10.0, 0.0), (30.0, 0.0), (30.0, 20.0), (10.0, 20.0)], 1);
    curves.push(ProfileCurve::circle(9, Point2::new(20.0, 10.0), 4.0));
    let profile = Profile::new(&curves).unwrap();
    let regions = profile.select(&Selection::EvenDepth).unwrap();
    let axis = Axis2::new(Point2::ZERO, Vector2::Y).unwrap();
    revolve(&Plane::XZ, &regions, axis, AngularExtent::full(), 1).unwrap()
}

pub fn spindle_rimmed() -> Solid {
    let curves = vec![
        ProfileCurve::line(1, Point2::new(0.0, 0.0), Point2::new(2.0, 0.0)),
        ProfileCurve::arc(
            2,
            Point2::new(2.0, 3.0),
            Point2::new(2.0, 0.0),
            Point2::new(5.0, 3.0),
        ),
        ProfileCurve::line(3, Point2::new(5.0, 3.0), Point2::new(5.0, 6.0)),
        ProfileCurve::line(4, Point2::new(5.0, 6.0), Point2::new(0.0, 6.0)),
        ProfileCurve::line(5, Point2::new(0.0, 6.0), Point2::new(0.0, 0.0)),
    ];
    let profile = Profile::new(&curves).unwrap();
    let regions = profile.select(&Selection::EvenDepth).unwrap();
    let axis = Axis2::new(Point2::ZERO, Vector2::Y).unwrap();
    revolve(&Plane::XZ, &regions, axis, AngularExtent::full(), 1).unwrap()
}

pub fn crossed_cylinders() -> Solid {
    let plane = Plane::from_frame(Point3::new(0.0, 0.0, -20.0), Vector3::Z, Vector3::X).unwrap();
    let upright = swept(plane, &[ProfileCurve::circle(1, Point2::ZERO, 8.0)], 40.0);
    let side = Plane::from_frame(Point3::new(-20.0, 0.0, 0.0), Vector3::X, Vector3::Y).unwrap();
    let across = swept(side, &[ProfileCurve::circle(1, Point2::ZERO, 5.0)], 40.0);
    boolean(&upright, &across, BooleanOperation::Union).unwrap()
}

pub fn spline_prism() -> Solid {
    let mut curves = vec![ProfileCurve::spline(
        1,
        3,
        vec![0.0, 0.0, 0.0, 0.0, 0.5, 1.0, 1.0, 1.0, 1.0],
        vec![
            Point2::new(0.0, 0.0),
            Point2::new(10.0, -6.0),
            Point2::new(20.0, 8.0),
            Point2::new(30.0, -4.0),
            Point2::new(40.0, 0.0),
        ],
    )];
    curves.push(ProfileCurve::line(
        2,
        Point2::new(40.0, 0.0),
        Point2::new(40.0, 20.0),
    ));
    curves.push(ProfileCurve::line(
        3,
        Point2::new(40.0, 20.0),
        Point2::new(0.0, 20.0),
    ));
    curves.push(ProfileCurve::line(
        4,
        Point2::new(0.0, 20.0),
        Point2::new(0.0, 0.0),
    ));
    swept(Plane::XY, &curves, 5.0)
}

pub fn filleted_block() -> Solid {
    let block = swept(
        Plane::XY,
        &polygon(&[(0.0, 0.0), (30.0, 0.0), (30.0, 20.0), (0.0, 20.0)], 1),
        15.0,
    );
    let edges: Vec<_> = block
        .edges()
        .filter(|(_, edge)| {
            let (start, end) = (
                block.vertex(edge.start()).unwrap().point(),
                block.vertex(edge.end()).unwrap().point(),
            );
            start.z == 15.0 && end.z == 15.0
        })
        .map(|(id, _)| id)
        .collect();
    blend(&block, &edges, BlendShape::Fillet { radius: 3.0 }, 2).unwrap()
}

pub fn all() -> Vec<(&'static str, Solid)> {
    vec![
        ("plate with hole", plate_with_hole()),
        ("turned", turned()),
        ("hollow ring", hollow_ring()),
        ("crossed cylinders", crossed_cylinders()),
        ("spline prism", spline_prism()),
        ("filleted block", filleted_block()),
    ]
}

pub fn volume(solid: &Solid) -> f64 {
    mesh_volume(
        &solid
            .tessellate(&SamplingTolerance::new(0.02, 0.2).unwrap())
            .unwrap(),
    )
}

pub fn mesh_volume(mesh: &Mesh) -> f64 {
    let origin = mesh.positions()[0];

    let mut total = 0.0;
    for triangle in mesh.triangles() {
        let corners = triangle.map(|index| mesh.vertices()[index as usize]);
        let [a, b, c] = corners.map(|corner| mesh.positions()[corner.position as usize] - origin);
        let flat = a.dot(b.cross(c)) / 6.0;
        let area = 0.5 * (b - a).cross(c - a).length();
        let sag = |from: usize, to: usize| {
            let (from, to) = (corners[from], corners[to]);
            let chord =
                mesh.positions()[to.position as usize] - mesh.positions()[from.position as usize];
            chord.dot(to.normal - from.normal) / 8.0
        };
        total += flat + area * (sag(0, 1) + sag(1, 2) + sag(2, 0)) / 3.0;
    }
    total
}
