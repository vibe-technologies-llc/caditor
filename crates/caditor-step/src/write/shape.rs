use std::collections::BTreeMap;

use caditor_geometry::{Plane, Point3, Vector3};
use caditor_kernel::{
    BSpline, BSplineSurface, Curve, EdgeId, FaceId, IntersectionCurve, Interval, ShellId, Solid,
    Surface, VertexId,
};

use crate::write::{Data, Ref, list, logical, text};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Unsupported {
    Geometry,
    Shells,
}

pub(crate) struct Shapes<'a> {
    data: &'a mut Data,
    vertices: BTreeMap<VertexId, Ref>,
    edges: BTreeMap<EdgeId, Ref>,
}

struct Lump {
    outer: ShellId,
    voids: Vec<ShellId>,
}

impl<'a> Shapes<'a> {
    pub fn new(data: &'a mut Data) -> Self {
        Self {
            data,
            vertices: BTreeMap::new(),
            edges: BTreeMap::new(),
        }
    }

    pub fn data(&mut self) -> &mut Data {
        self.data
    }

    pub fn origin(data: &mut Data) -> Ref {
        placement(data, Point3::ZERO, Vector3::Z, Vector3::X)
    }

    pub fn body(&mut self, solid: &Solid, name: &str) -> Result<Vec<Ref>, Unsupported> {
        self.vertices.clear();
        self.edges.clear();
        let mut solids = Vec::new();
        for lump in lumps(solid)? {
            let outer = self.closed_shell(solid, lump.outer, false)?;
            let entity = if lump.voids.is_empty() {
                format!("MANIFOLD_SOLID_BREP({},{outer})", text(name))
            } else {
                let mut voids = Vec::new();
                for void in lump.voids {
                    let shell = self.closed_shell(solid, void, true)?;
                    voids.push(
                        self.data
                            .add(format!("ORIENTED_CLOSED_SHELL('',*,{shell},.F.)")),
                    );
                }
                format!("BREP_WITH_VOIDS({},{outer},{})", text(name), list(voids))
            };
            solids.push(self.data.add(entity));
        }
        Ok(solids)
    }

    fn closed_shell(
        &mut self,
        solid: &Solid,
        shell: ShellId,
        flipped: bool,
    ) -> Result<Ref, Unsupported> {
        let faces = solid.shell(shell).ok_or(Unsupported::Geometry)?.faces();
        let mut written = Vec::with_capacity(faces.len());
        for face in faces {
            written.push(self.face(solid, *face, flipped)?);
        }
        Ok(self.data.add(format!("CLOSED_SHELL('',{})", list(written))))
    }

    fn face(&mut self, solid: &Solid, id: FaceId, flipped: bool) -> Result<Ref, Unsupported> {
        let face = solid.face(id).ok_or(Unsupported::Geometry)?;
        let mut bounds = Vec::with_capacity(face.loops().len());
        for (index, loop_id) in face.loops().iter().enumerate() {
            let face_loop = solid.face_loop(*loop_id).ok_or(Unsupported::Geometry)?;
            let mut oriented = Vec::with_capacity(face_loop.coedges().len());
            for coedge_id in face_loop.coedges() {
                let coedge = solid.coedge(*coedge_id).ok_or(Unsupported::Geometry)?;
                let edge = self.edge(solid, coedge.edge())?;
                oriented.push(self.data.add(format!(
                    "ORIENTED_EDGE('',*,*,{edge},{})",
                    logical(coedge.sense().is_same())
                )));
            }
            let edge_loop = self.data.add(format!("EDGE_LOOP('',{})", list(oriented)));
            let kind = if index == 0 {
                "FACE_OUTER_BOUND"
            } else {
                "FACE_BOUND"
            };
            bounds.push(
                self.data
                    .add(format!("{kind}('',{edge_loop},{})", logical(!flipped))),
            );
        }
        let surface = self.surface(face.surface())?;
        Ok(self.data.add(format!(
            "ADVANCED_FACE('',{},{surface},{})",
            list(bounds),
            logical(face.sense().is_same() != flipped)
        )))
    }

    fn vertex(&mut self, solid: &Solid, id: VertexId) -> Result<Ref, Unsupported> {
        if let Some(existing) = self.vertices.get(&id) {
            return Ok(*existing);
        }
        let point = point(
            self.data,
            solid.vertex(id).ok_or(Unsupported::Geometry)?.point(),
        );
        let vertex = self.data.add(format!("VERTEX_POINT('',{point})"));
        self.vertices.insert(id, vertex);
        Ok(vertex)
    }

    fn edge(&mut self, solid: &Solid, id: EdgeId) -> Result<Ref, Unsupported> {
        if let Some(existing) = self.edges.get(&id) {
            return Ok(*existing);
        }
        let edge = solid.edge(id).ok_or(Unsupported::Geometry)?;
        let start = self.vertex(solid, edge.start())?;
        let end = self.vertex(solid, edge.end())?;
        let ends = [edge.start(), edge.end()]
            .map(|vertex| solid.vertex(vertex).map(|vertex| vertex.point()));
        let ends = match ends {
            [Some(start), Some(end)] => Some((start, end)),
            _ => None,
        };
        let curve = self.curve(edge.curve(), Some(edge.interval()), ends)?;
        let written = self
            .data
            .add(format!("EDGE_CURVE('',{start},{end},{curve},.T.)"));
        self.edges.insert(id, written);
        Ok(written)
    }

    fn curve(
        &mut self,
        curve: &Curve,
        interval: Option<Interval>,
        ends: Option<(Point3, Point3)>,
    ) -> Result<Ref, Unsupported> {
        Ok(match curve {
            Curve::Line(line) => {
                let origin = point(self.data, line.origin());
                let direction = direction(self.data, line.direction());
                let vector = self.data.add(format!("VECTOR('',{direction},1.)"));
                self.data.add(format!("LINE('',{origin},{vector})"))
            }
            Curve::Circle(circle) => {
                let frame = frame_placement(self.data, circle.frame());
                let radius = self.data.real(circle.radius());
                self.data.add(format!("CIRCLE('',{frame},{radius})"))
            }
            Curve::Ellipse(ellipse) => {
                let frame = frame_placement(self.data, ellipse.frame());
                let major = self.data.real(ellipse.major_radius());
                let minor = self.data.real(ellipse.minor_radius());
                self.data
                    .add(format!("ELLIPSE('',{frame},{major},{minor})"))
            }
            Curve::BSpline(spline) => self.spline(spline),
            Curve::Intersection(intersection) => {
                let range = interval.unwrap_or_else(|| intersection.domain());
                let trimmed = intersection.trimmed(range).ok_or(Unsupported::Geometry)?;
                let spline = hermite_spline(&trimmed, ends).ok_or(Unsupported::Geometry)?;
                self.spline(&spline)
            }
            _ => return Err(Unsupported::Geometry),
        })
    }

    fn spline(&mut self, spline: &BSpline<Point3>) -> Ref {
        let points: Vec<Ref> = spline
            .control_points()
            .iter()
            .map(|control| point(self.data, *control))
            .collect();
        let (multiplicities, knots) = knot_runs(spline.knots());
        let degree = spline.degree();
        let points = list(points);
        let multiplicities = list(multiplicities);
        let knots = self.data.reals(knots);
        match spline.weights() {
            None => self.data.add(format!(
                "B_SPLINE_CURVE_WITH_KNOTS('',{degree},{points},.UNSPECIFIED.,.F.,.F.,{multiplicities},{knots},.UNSPECIFIED.)"
            )),
            Some(weights) => {
                let weights = self.data.reals(weights.iter().copied());
                self.data.add(format!(
                    "(BOUNDED_CURVE() B_SPLINE_CURVE({degree},{points},.UNSPECIFIED.,.F.,.F.) B_SPLINE_CURVE_WITH_KNOTS({multiplicities},{knots},.UNSPECIFIED.) CURVE() GEOMETRIC_REPRESENTATION_ITEM() RATIONAL_B_SPLINE_CURVE({weights}) REPRESENTATION_ITEM(''))"
                ))
            }
        }
    }

    fn spline_surface(&mut self, spline: &BSplineSurface) -> Ref {
        let mut grid = Vec::with_capacity(spline.columns());
        let mut weights = Vec::with_capacity(spline.columns());
        for column in 0..spline.columns() {
            let mut row = Vec::with_capacity(spline.rows());
            let mut row_weights = Vec::with_capacity(spline.rows());
            for index in 0..spline.rows() {
                let control = spline.control_point(column, index).unwrap_or(Point3::ZERO);
                row.push(point(self.data, control));
                row_weights.push(
                    spline
                        .weights()
                        .and_then(|weights| weights.get(index * spline.columns() + column))
                        .copied()
                        .unwrap_or(1.0),
                );
            }
            grid.push(list(row));
            weights.push(self.data.reals(row_weights));
        }
        let (u_multiplicities, u_knots) = knot_runs(spline.u_knots());
        let (v_multiplicities, v_knots) = knot_runs(spline.v_knots());
        let (u_degree, v_degree) = (spline.u_degree(), spline.v_degree());
        let grid = list(grid);
        let knots = format!(
            "{},{},{},{}",
            list(u_multiplicities),
            list(v_multiplicities),
            self.data.reals(u_knots),
            self.data.reals(v_knots)
        );
        match spline.weights() {
            None => self.data.add(format!(
                "B_SPLINE_SURFACE_WITH_KNOTS('',{u_degree},{v_degree},{grid},.UNSPECIFIED.,.F.,.F.,.F.,{knots},.UNSPECIFIED.)"
            )),
            Some(_) => self.data.add(format!(
                "(BOUNDED_SURFACE() B_SPLINE_SURFACE({u_degree},{v_degree},{grid},.UNSPECIFIED.,.F.,.F.,.F.) B_SPLINE_SURFACE_WITH_KNOTS({knots},.UNSPECIFIED.) GEOMETRIC_REPRESENTATION_ITEM() RATIONAL_B_SPLINE_SURFACE({}) REPRESENTATION_ITEM('') SURFACE())",
                list(weights)
            )),
        }
    }

    fn surface(&mut self, surface: &Surface) -> Result<Ref, Unsupported> {
        Ok(match surface {
            Surface::Plane(plane) => {
                let frame = frame_placement(self.data, plane.frame());
                self.data.add(format!("PLANE('',{frame})"))
            }
            Surface::Cylinder(cylinder) => {
                let frame = frame_placement(self.data, cylinder.frame());
                let radius = self.data.real(cylinder.radius());
                self.data
                    .add(format!("CYLINDRICAL_SURFACE('',{frame},{radius})"))
            }
            Surface::Cone(cone) => {
                let source = cone.frame();
                let opening = cone.half_angle();
                let frame = if opening > 0.0 {
                    frame_placement(self.data, source)
                } else {
                    placement(
                        self.data,
                        source.origin(),
                        -source.normal(),
                        source.x_axis(),
                    )
                };
                let radius = self.data.real(cone.radius());
                let angle = self.data.real(opening.abs());
                self.data
                    .add(format!("CONICAL_SURFACE('',{frame},{radius},{angle})"))
            }
            Surface::Sphere(sphere) => {
                let frame = frame_placement(self.data, sphere.frame());
                let radius = self.data.real(sphere.radius());
                self.data
                    .add(format!("SPHERICAL_SURFACE('',{frame},{radius})"))
            }
            Surface::Torus(torus) => {
                let frame = frame_placement(self.data, torus.frame());
                let major = self.data.real(torus.major_radius());
                let minor = self.data.real(torus.minor_radius());
                self.data
                    .add(format!("TOROIDAL_SURFACE('',{frame},{major},{minor})"))
            }
            Surface::Extrusion(extrusion) => {
                let profile = self.curve(extrusion.profile(), None, None)?;
                let direction = direction(self.data, extrusion.direction());
                let vector = self.data.add(format!("VECTOR('',{direction},1.)"));
                self.data.add(format!(
                    "SURFACE_OF_LINEAR_EXTRUSION('',{profile},{vector})"
                ))
            }
            Surface::BSpline(spline) => self.spline_surface(spline),
            Surface::Revolution(revolution) => {
                let profile = self.curve(revolution.profile(), None, None)?;
                let origin = point(self.data, revolution.axis_origin());
                let axis = direction(self.data, revolution.axis_direction());
                let placement = self
                    .data
                    .add(format!("AXIS1_PLACEMENT('',{origin},{axis})"));
                self.data
                    .add(format!("SURFACE_OF_REVOLUTION('',{profile},{placement})"))
            }
            _ => return Err(Unsupported::Geometry),
        })
    }
}

fn point(data: &mut Data, point: Point3) -> Ref {
    let coordinates = data.reals([point.x, point.y, point.z]);
    data.add(format!("CARTESIAN_POINT('',{coordinates})"))
}

fn direction(data: &mut Data, direction: Vector3) -> Ref {
    let unit = direction.normalize_or_zero();
    let coordinates = data.reals([unit.x, unit.y, unit.z]);
    data.add(format!("DIRECTION('',{coordinates})"))
}

fn placement(data: &mut Data, origin: Point3, axis: Vector3, reference: Vector3) -> Ref {
    let origin = point(data, origin);
    let axis = direction(data, axis);
    let reference = direction(data, reference);
    data.add(format!(
        "AXIS2_PLACEMENT_3D('',{origin},{axis},{reference})"
    ))
}

fn frame_placement(data: &mut Data, frame: &Plane) -> Ref {
    placement(data, frame.origin(), frame.normal(), frame.x_axis())
}

fn knot_runs(knots: &[f64]) -> (Vec<usize>, Vec<f64>) {
    let mut multiplicities: Vec<usize> = Vec::new();
    let mut distinct: Vec<f64> = Vec::new();
    for knot in knots {
        match (distinct.last(), multiplicities.last_mut()) {
            (Some(last), Some(count)) if last == knot => *count += 1,
            _ => {
                distinct.push(*knot);
                multiplicities.push(1);
            }
        }
    }
    (multiplicities, distinct)
}

fn hermite_spline(
    curve: &IntersectionCurve,
    ends: Option<(Point3, Point3)>,
) -> Option<BSpline<Point3>> {
    let nodes = curve.nodes();
    let first = nodes.first()?;
    let mut points = vec![first.point];
    let mut knots = vec![first.parameter; 4];
    for pair in nodes.windows(2) {
        let [start, end] = pair else {
            continue;
        };
        let third = (end.parameter - start.parameter) / 3.0;
        points.extend([
            start.point + start.derivative * third,
            end.point - end.derivative * third,
            end.point,
        ]);
        knots.extend([end.parameter; 3]);
    }
    knots.push(nodes.last()?.parameter);
    if let Some((start, end)) = ends {
        if let Some(first) = points.first_mut() {
            *first = start;
        }
        if let Some(last) = points.last_mut() {
            *last = end;
        }
    }
    BSpline::new(3, knots, points).ok()
}

struct Classified {
    outward: Vec<ShellId>,
    inward: Vec<ShellId>,
}

fn classify(solid: &Solid) -> Option<(Classified, caditor_kernel::Mesh)> {
    let mesh = solid.tessellate(&solid.default_tolerance()).ok()?;
    let mut classified = Classified {
        outward: Vec::new(),
        inward: Vec::new(),
    };
    for (id, _) in solid.shells() {
        let in_shell = |face: FaceId| solid.face(face).is_some_and(|face| face.shell() == id);
        if mesh.mass_properties_where(in_shell).volume < 0.0 {
            classified.inward.push(id);
        } else {
            classified.outward.push(id);
        }
    }
    Some((classified, mesh))
}

fn lumps(solid: &Solid) -> Result<Vec<Lump>, Unsupported> {
    let shells: Vec<ShellId> = solid.shells().map(|(id, _)| id).collect();
    if let [outer] = shells.as_slice() {
        return Ok(vec![Lump {
            outer: *outer,
            voids: Vec::new(),
        }]);
    }
    let Some((classified, mesh)) = classify(solid) else {
        log::warn!("the solid could not be meshed, so its shells cannot be told apart");
        return Err(Unsupported::Shells);
    };
    let mut lumps: Vec<Lump> = classified
        .outward
        .iter()
        .map(|outer| Lump {
            outer: *outer,
            voids: Vec::new(),
        })
        .collect();
    for void in classified.inward {
        let probe = shell_point(solid, void);
        let owner = lumps.iter().position(|lump| {
            probe.is_some_and(|probe| {
                mesh.contains(probe, |face| {
                    solid
                        .face(face)
                        .is_some_and(|face| face.shell() == lump.outer)
                }) == Some(true)
            })
        });
        let lump = owner
            .and_then(|owner| lumps.get_mut(owner))
            .ok_or(Unsupported::Shells)?;
        lump.voids.push(void);
    }
    Ok(lumps)
}

fn shell_point(solid: &Solid, shell: ShellId) -> Option<Point3> {
    let face = solid.face(*solid.shell(shell)?.faces().first()?)?;
    let face_loop = solid.face_loop(face.outer_loop()?)?;
    let coedge = solid.coedge(*face_loop.coedges().first()?)?;
    let edge = solid.edge(coedge.edge())?;
    Some(solid.vertex(edge.start())?.point())
}
