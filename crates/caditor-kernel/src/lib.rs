mod blend;
mod boolean;
mod box_tree;
mod bspline;
mod build;
mod checks;
mod coordinates;
mod curve;
mod curve2;
mod error;
mod faceted;
#[cfg(test)]
mod fixtures;
mod hole_faces;
mod interrupt;
mod intersect;
mod interval;
mod mapping;
mod measure;
mod naming;
mod numeric;
mod parametric;
mod pattern;
mod profile;
mod sense;
mod shell;
mod surface;
mod tessellation;
#[cfg(test)]
mod test_support;
mod tolerance;
mod topology;

pub use crate::{
    blend::{BlendError, BlendShape, blend, blend_chain, tangent_chain, tangent_faces},
    boolean::{BooleanError, BooleanOperation, BooleanSite, Interference, boolean, interference},
    bspline::{BSpline, MAX_SPLINE_DEGREE},
    build::{
        AngularExtent, Axis2, Heights, LinearBound, LinearExtent, MAX_TAPER_DEGREES, NextFace,
        ReachError, StopError, Stopped, SweepError, extrude, extrude_along, extrude_tapered,
        heights, heights_along, next_face, revolve, stop_at_body,
    },
    coordinates::Coordinates,
    curve::{
        BSplineCurve, Circle, Curve, CurveDerivatives, CurveSample, Ellipse, IntersectionCurve,
        IntersectionNode, Line,
    },
    curve2::{BSplineCurve2, Circle2, Curve2, Curve2Derivatives, Curve2Sample, Ellipse2, Line2},
    error::GeometryError,
    faceted::{
        FacetedError, FacetedSolids, MAX_FACETED_FACES, MAX_FILLED_HOLE_EDGES, MeshRepairs,
        TriangleMesh, faceted_solids,
    },
    hole_faces::hole_faces,
    interrupt::{Interrupt, Interrupted, check as check_interrupt, interruptible},
    intersect::{
        CurveCurveIntersection, CurveCurveOverlap, CurveCurvePoint, CurveSurfaceIntersection,
        CurveSurfaceOverlap, CurveSurfacePoint, IntersectionBranch, IntersectionError,
        IntersectionPoint, SurfaceIntersection, SurfacePatch, intersect_curve_surface,
        intersect_curves, intersect_curves2, intersect_surfaces,
    },
    interval::{Domain, Interval},
    measure::{
        Accuracy, Angle, AngleKind, Axis, EdgeForm, EdgeMeasure, Element, FaceForm, MeasureError,
        Separation, SolidMass, angle, axis_of, axis_separation, curve_measure, distance,
        edge_measure, extent, face_area, face_form, mass_properties,
    },
    naming::{
        EdgeName, EdgeNaming, EdgeReference, FaceCopy, FaceName, FaceOrigin, FaceReference, Made,
        ReferenceError, VertexName, vertex_names,
    },
    pattern::{PatternCopy, PatternError, pattern, pattern_copies},
    profile::{
        AreaMoments, BoundaryPiece, Neighbour, OpenEnd, Piece, PieceBound, PieceId,
        PrincipalMoments, Profile, ProfileCurve, ProfileError, ProfileLoop, ProfileShape, Region,
        RegionKey, RegionMatch, RegionMesh, RegionReference, ResolvedRegions, Section, Selection,
        Side, WallError, WallSide, resolve_regions, section_of, wall_regions,
    },
    sense::Sense,
    shell::{OffsetError, ShellError, offset_faces, shell},
    surface::{
        BSplineSurface, BendError, BendTarget, BentSide, Cone, Cylinder, Extrusion,
        MAX_BENT_CONTROL_POINTS, MAX_SIDE_CONTROL_POINTS, PlaneSurface, Pole, Revolution, SideBend,
        Sphere, Surface, SurfaceDerivatives, SurfaceSide, Torus,
    },
    tessellation::{
        EdgePolyline, FaceTriangles, MassProperties, Mesh, MeshVertex, SecondMoment,
        TessellationError,
    },
    tolerance::{
        ANGULAR_RESOLUTION, INTERSECTION_TOLERANCE, LINEAR_RESOLUTION, MAX_SIZE, MODEL_EXTENT,
        MeshQuality, PCURVE_TOLERANCE, SamplingTolerance, parallel, same_direction, same_point,
    },
    topology::{
        Along, BoundaryClass, BuildError, Coedge, CoedgeId, Crossing, CrossingCheck, Edge, EdgeId,
        Face, FaceContainment, FaceId, IsoparametricError, IsoparametricRun, Loop, LoopId,
        MOST_ISOPARAMETRIC_LINES, Pcurve, PcurveError, PcurveSample, PointClass, RayCrossing,
        Shell, ShellId, Solid, SolidBuilder, SolidClassifier, TransformError, ValidationError,
        Vertex, VertexId,
    },
};
