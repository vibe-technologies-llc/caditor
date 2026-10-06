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
    blend::{BlendError, BlendShape, blend, blend_chain},
    boolean::{BooleanError, BooleanOperation, BooleanSite, Interference, boolean, interference},
    bspline::{BSpline, MAX_SPLINE_DEGREE},
    build::{
        AngularExtent, Axis2, Heights, LinearBound, LinearExtent, NextFace, ReachError, SweepError,
        extrude, heights, next_face, revolve,
    },
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
    measure::{
        Accuracy, Angle, AngleKind, Axis, EdgeForm, EdgeMeasure, Element, FaceForm, MeasureError,
        Separation, angle, axis_of, axis_separation, distance, edge_measure, face_form,
        planar_area,
    },
    naming::{
        EdgeName, EdgeNaming, EdgeReference, FaceCopy, FaceName, FaceOrigin, FaceReference, Made,
        ReferenceError, VertexName, vertex_names,
    },
    pattern::{PatternCopy, PatternError, pattern},
    profile::{
        BoundaryPiece, Neighbour, OpenEnd, Piece, PieceBound, PieceId, Profile, ProfileCurve,
        ProfileError, ProfileLoop, ProfileShape, Region, RegionKey, RegionMatch, RegionMesh,
        RegionReference, ResolvedRegions, Selection, Side, resolve_regions,
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
        MeshQuality, PCURVE_TOLERANCE, SamplingTolerance, parallel, same_direction, same_point,
    },
    topology::{
        BoundaryClass, BuildError, Coedge, CoedgeId, Crossing, CrossingCheck, Edge, EdgeId, Face,
        FaceContainment, FaceId, Loop, LoopId, Pcurve, PcurveError, PcurveSample, PointClass,
        RayCrossing, Shell, ShellId, Solid, SolidBuilder, SolidClassifier, TransformError,
        ValidationError, Vertex, VertexId,
    },
};
