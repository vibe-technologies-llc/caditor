mod blend;
mod boolean;
mod box_tree;
mod bspline;
mod build;
mod checks;
mod coil;
mod coordinates;
mod curve;
mod curve2;
mod enclose;
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
mod shared;
mod shell;
mod surface;
mod tally;
mod tessellation;
#[cfg(test)]
mod test_support;
mod thread;
mod tolerance;
mod topology;

pub use crate::{
    blend::{
        BlendError, BlendShape, blend, blend_chain, measured_face, tangent_chain, tangent_faces,
    },
    boolean::{
        BooleanError, BooleanOperation, BooleanSite, FaceSplitError, Interference, boolean,
        interference, split_faces,
    },
    bspline::{BSpline, MAX_SPLINE_DEGREE},
    build::{
        AngularExtent, Axis2, BendMiddle, BendMiddles, Heights, LinearBound, LinearExtent,
        LoftEnds, LoftError, LoftProfile, LoftSkin, MAX_LOFT_PROFILES, MAX_TAPER_DEGREES,
        MIN_FLANK_DEGREES, NextFace, PathSegment, PathSweepError, PipeCorner, PipeError,
        PipeSection, PipeShape, PipeStand, ReachError, StopError, Stopped, SweepError,
        SweepOrientation, SweepShape, WindingAxis, WrapBand, WrapError, extrude, extrude_along,
        extrude_flared, extrude_tapered, extrude_tapered_along, heights, heights_along, loft,
        loft_through, next_face, pipe, pipe_stand, revolve, stop_at_body, sweep, sweep_shaped,
        wrap_chain, wrap_regions, wrap_regions_within, wrapped_stand,
    },
    coil::{
        COIL_PARTS, CoilError, CoilFace, CoilParts, CoilPlacement, CoilShape, CoilSpec, HelixError,
        HelixSpec, HelixTurn, LEAST_COIL_TURNS, MAX_COIL_TAPER_DEGREES, MAX_COIL_TURNS, coil,
        helix,
    },
    coordinates::Coordinates,
    curve::{
        BSplineCurve, Circle, Curve, CurveDerivatives, CurveSample, Ellipse, IntersectionCurve,
        IntersectionNode, Line,
    },
    curve2::{BSplineCurve2, Circle2, Curve2, Curve2Derivatives, Curve2Sample, Ellipse2, Line2},
    enclose::{Bounds, Enclosure, EnclosureError, enclose_faces},
    error::GeometryError,
    faceted::{
        FaceMesh, FacetedError, FacetedSolids, MAX_FACETED_FACES, MAX_FILLED_HOLE_EDGES,
        MeshRepairs, TriangleMesh, faceted_solids,
    },
    hole_faces::hole_faces,
    interrupt::{
        Interrupt, Interrupted, check as check_interrupt, current as current_interrupt,
        interruptible,
    },
    intersect::{
        CurveCurveIntersection, CurveCurveOverlap, CurveCurvePoint, CurveSurfaceIntersection,
        CurveSurfaceOverlap, CurveSurfacePoint, IntersectionBranch, IntersectionError,
        IntersectionPoint, RefineStop, SurfaceIntersection, SurfacePatch, intersect_curve_surface,
        intersect_curves, intersect_curves2, intersect_surfaces,
    },
    interval::{Domain, Interval},
    measure::{
        Accuracy, Angle, AngleKind, Axis, EdgeForm, EdgeMeasure, Element, FaceForm, MeasureError,
        Separation, SolidMass, angle, axis_of, axis_separation, curve_measure, distance,
        edge_measure, extent, face_area, face_form, mass_properties, mass_properties_tessellating,
    },
    naming::{
        EdgeName, EdgeNaming, EdgeReference, FaceCopy, FaceName, FaceOrigin, FaceReference, Made,
        ReferenceError, SplitPiece, VertexName, faces_around_vertices, vertex_names,
    },
    pattern::{PatternCopy, PatternError, blend_copy, pattern, pattern_copies, shell_copies},
    profile::{
        AreaMoments, BoundaryPiece, Neighbour, OpenEnd, Piece, PieceBound, PieceId,
        PrincipalMoments, Profile, ProfileCurve, ProfileError, ProfileLoop, ProfileShape, Region,
        RegionKey, RegionMatch, RegionMesh, RegionReference, ResolvedRegions, Section, Selection,
        Side, WallError, WallSide, resolve_regions, section_of, wall_regions,
    },
    sense::Sense,
    shared::SharedBuffer,
    shell::{
        DeleteError, DraftError, DraftedFace, MoveError, OffsetError, ReplaceError, STEEPEST_DRAFT,
        STEEPEST_REPLACING_TURN, ShellError, ShellSide, Walls, check_draftable, delete_faces,
        draft_faces, move_faces, offset_faces, replace_faces, shell, shell_walls,
    },
    surface::{
        BSplineSurface, BendError, BendTarget, BentSide, Cone, Cylinder, Extrusion,
        MAX_BENT_CONTROL_POINTS, MAX_SIDE_CONTROL_POINTS, PlaneSurface, Pole, Revolution, SideBend,
        Sphere, Surface, SurfaceDerivatives, SurfaceSide, Torus,
    },
    tessellation::{
        DisplayMesh, EdgePolyline, FaceTriangles, MassProperties, Mesh, MeshNormal, MeshVertex,
        MeshedSolid, SecondMoment, TessellationError,
    },
    thread::{
        GROOVE_PARTS, Handedness, MAX_THREAD_TURNS, RING_ENTITY, SPANS_PER_TURN, ThreadEnd,
        ThreadError, ThreadForm, ThreadSpec, ThreadedFace, thread, thread_tools,
        thread_tools_numbered,
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
