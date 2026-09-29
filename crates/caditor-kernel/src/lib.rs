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
#[cfg(test)]
mod fixtures;
mod interrupt;
mod intersect;
mod interval;
mod naming;
mod numeric;
mod parametric;
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
    blend::{BlendError, BlendShape, blend, blend_chain},
    boolean::{BooleanError, BooleanOperation, boolean},
    bspline::{BSpline, MAX_SPLINE_DEGREE},
    build::{AngularExtent, Axis2, LinearExtent, SweepError, extrude, revolve},
    coordinates::Coordinates,
    curve::{
        BSplineCurve, Circle, Curve, CurveDerivatives, CurveSample, Ellipse, IntersectionCurve,
        IntersectionNode, Line,
    },
    curve2::{BSplineCurve2, Circle2, Curve2, Curve2Derivatives, Curve2Sample, Line2},
    error::GeometryError,
    interrupt::{Interrupt, Interrupted, interruptible},
    intersect::{
        CurveCurveIntersection, CurveCurveOverlap, CurveCurvePoint, CurveSurfaceIntersection,
        CurveSurfaceOverlap, CurveSurfacePoint, IntersectionBranch, IntersectionError,
        IntersectionPoint, SurfaceIntersection, SurfacePatch, intersect_curve_surface,
        intersect_curves, intersect_curves2, intersect_surfaces,
    },
    interval::{Domain, Interval},
    naming::{
        EdgeName, EdgeNaming, EdgeReference, FaceName, FaceOrigin, FaceReference, ReferenceError,
        VertexName,
    },
    profile::{
        Piece, PieceBound, PieceId, Profile, ProfileCurve, ProfileError, ProfileLoop, ProfileShape,
        Region, RegionKey, RegionMesh, Selection, Side,
    },
    sense::Sense,
    shell::{ShellError, shell},
    surface::{
        BSplineSurface, Cone, Cylinder, Extrusion, PlaneSurface, Pole, Revolution, Sphere, Surface,
        SurfaceDerivatives, Torus,
    },
    tessellation::{
        EdgePolyline, FaceTriangles, MassProperties, Mesh, MeshVertex, TessellationError,
    },
    tolerance::{
        ANGULAR_RESOLUTION, INTERSECTION_TOLERANCE, LINEAR_RESOLUTION, MAX_SIZE, MODEL_EXTENT,
        PCURVE_TOLERANCE, SamplingTolerance, parallel, same_direction, same_point,
    },
    topology::{
        BoundaryClass, BuildError, Coedge, CoedgeId, Crossing, CrossingCheck, Edge, EdgeId, Face,
        FaceContainment, FaceId, Loop, LoopId, Pcurve, PcurveError, PcurveSample, PointClass,
        Shell, ShellId, Solid, SolidBuilder, SolidClassifier, ValidationError, Vertex, VertexId,
    },
};
