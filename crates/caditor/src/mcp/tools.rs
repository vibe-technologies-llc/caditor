use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};

use crate::{
    mcp::{
        curves::{self, CurveToolError},
        datum_schemas,
        datums::{self, DatumError},
        edges::{self, MeasuringError},
        edits::{self, FeatureEditError},
        feature_schemas,
        features::{self, FeatureError},
        files::{self, FileError},
        mates::{self, MateError},
        places::PlaceError,
        reading::{self, RenderError},
        references::ReferenceError,
        reshaping::{self, ReshapeError},
        session::{Cancelled, ChangeError, Session, Work},
        sketch_edits::{self, SketchEditError},
        sketch_schemas,
        sketching::{self, SketchToolError},
        solids, sweep_schemas,
        sweeps::{self, SweepToolError},
        tree_edits::{self, TreeError},
        writing::{self, ModellingError, SketchingError},
    },
    variants::all_variants,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tool {
    GetModel,
    ListFeatures,
    ListParameters,
    ListBodies,
    ListFaces,
    GetSketch,
    RenderView,
    SetParameter,
    AddSketch,
    AddSketchGeometry,
    AddSketchConstraints,
    RemoveSketchItems,
    EditSketchConstraint,
    ModifySketch,
    AddExtrusion,
    AddHole,
    Undo,
    Redo,
    Save,
    Export,
    ListEdges,
    Measure,
    AddRevolve,
    AddFillet,
    AddChamfer,
    AddShell,
    AddOffsetFace,
    AddPrimitive,
    AddMirror,
    AddLinearPattern,
    AddCircularPattern,
    AddCombine,
    AddMove,
    AddDatumPlane,
    AddDatumAxis,
    AddSweep,
    AddPipe,
    AddLoft,
    AddCoil,
    AddRib,
    AddThread,
    AddSplitFace,
    AddDeleteFace,
    AddEmboss,
    AddMoveFace,
    AddDraft,
    AddReplaceFace,
    AddSplit,
    AddScale,
    AddDatumPoint,
    AddCoordinateSystem,
    AddMate,
    AddHelix,
    AddCurvePattern,
    AddPointPattern,
    Import,
    SaveAs,
    EditFeature,
    RenameFeature,
    SuppressFeatures,
    MoveFeature,
    DeleteFeatures,
}

all_variants!(
    Tool: GetModel,
    ListFeatures,
    ListParameters,
    ListBodies,
    ListFaces,
    GetSketch,
    RenderView,
    SetParameter,
    AddSketch,
    AddSketchGeometry,
    AddSketchConstraints,
    RemoveSketchItems,
    EditSketchConstraint,
    ModifySketch,
    AddExtrusion,
    AddHole,
    Undo,
    Redo,
    Save,
    Export,
    ListEdges,
    Measure,
    AddRevolve,
    AddFillet,
    AddChamfer,
    AddShell,
    AddOffsetFace,
    AddPrimitive,
    AddMirror,
    AddLinearPattern,
    AddCircularPattern,
    AddCombine,
    AddMove,
    AddDatumPlane,
    AddDatumAxis,
    AddSweep,
    AddPipe,
    AddLoft,
    AddCoil,
    AddRib,
    AddThread,
    AddSplitFace,
    AddDeleteFace,
    AddEmboss,
    AddMoveFace,
    AddDraft,
    AddReplaceFace,
    AddSplit,
    AddScale,
    AddDatumPoint,
    AddCoordinateSystem,
    AddMate,
    AddHelix,
    AddCurvePattern,
    AddPointPattern,
    Import,
    SaveAs,
    EditFeature,
    RenameFeature,
    SuppressFeatures,
    MoveFeature,
    DeleteFeatures
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Effect {
    Reads,
    Adds,
    Changes,
    Writes,
}

pub struct Answer {
    pub data: Map<String, Value>,
    pub image: Option<Vec<u8>>,
}

impl Answer {
    pub fn of(data: Map<String, Value>) -> Self {
        Self { data, image: None }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ValueRefusal {
    #[error(transparent)]
    Expression(#[from] crate::field::ExpressionError),
    #[error(transparent)]
    Dimension(#[from] crate::field::DimensionEditError),
    #[error("{}", crate::field::sentence(&.0.to_string()))]
    Unevaluable(caditor_expression::EvalError),
    #[error(transparent)]
    Name(#[from] caditor_expression::NameError),
}

#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("the arguments do not fit {tool}: {source}")]
    Arguments {
        tool: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Reference(#[from] ReferenceError),
    #[error("{field} “{text}” cannot be used: {reason}")]
    Value {
        field: &'static str,
        text: String,
        reason: Box<ValueRefusal>,
    },
    #[error(transparent)]
    Change(#[from] ChangeError),
    #[error(transparent)]
    Sketching(#[from] SketchingError),
    #[error(transparent)]
    SketchTools(#[from] SketchToolError),
    #[error(transparent)]
    SketchEdits(#[from] SketchEditError),
    #[error(transparent)]
    Modelling(#[from] ModellingError),
    #[error(transparent)]
    Files(#[from] FileError),
    #[error(transparent)]
    Render(#[from] RenderError),
    #[error(transparent)]
    Cancelled(#[from] Cancelled),
    #[error(transparent)]
    Place(#[from] PlaceError),
    #[error(transparent)]
    Feature(#[from] FeatureError),
    #[error(transparent)]
    Measuring(#[from] MeasuringError),
    #[error(transparent)]
    Tree(#[from] TreeError),
    #[error(transparent)]
    Sweeps(#[from] SweepToolError),
    #[error(transparent)]
    Reshaping(#[from] ReshapeError),
    #[error(transparent)]
    Editing(#[from] FeatureEditError),
    #[error(transparent)]
    Datums(#[from] DatumError),
    #[error(transparent)]
    Mates(#[from] MateError),
    #[error(transparent)]
    Curves(#[from] CurveToolError),
}

impl Tool {
    pub fn named(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|tool| tool.name() == name)
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::GetModel => "get_model",
            Self::ListFeatures => "list_features",
            Self::ListParameters => "list_parameters",
            Self::ListBodies => "list_bodies",
            Self::ListFaces => "list_faces",
            Self::GetSketch => "get_sketch",
            Self::RenderView => "render_view",
            Self::SetParameter => "set_parameter",
            Self::AddSketch => "add_sketch",
            Self::AddSketchGeometry => "add_sketch_geometry",
            Self::AddSketchConstraints => "add_sketch_constraints",
            Self::RemoveSketchItems => "remove_sketch_items",
            Self::EditSketchConstraint => "edit_sketch_constraint",
            Self::ModifySketch => "modify_sketch",
            Self::AddExtrusion => "add_extrusion",
            Self::AddHole => "add_hole",
            Self::Undo => "undo",
            Self::Redo => "redo",
            Self::Save => "save",
            Self::Export => "export",
            Self::ListEdges => "list_edges",
            Self::Measure => "measure",
            Self::AddRevolve => "add_revolve",
            Self::AddFillet => "add_fillet",
            Self::AddChamfer => "add_chamfer",
            Self::AddShell => "add_shell",
            Self::AddOffsetFace => "add_offset_face",
            Self::AddPrimitive => "add_primitive",
            Self::AddMirror => "add_mirror",
            Self::AddLinearPattern => "add_linear_pattern",
            Self::AddCircularPattern => "add_circular_pattern",
            Self::AddCombine => "add_combine",
            Self::AddMove => "add_move",
            Self::AddDatumPlane => "add_datum_plane",
            Self::AddDatumAxis => "add_datum_axis",
            Self::AddSweep => "add_sweep",
            Self::AddPipe => "add_pipe",
            Self::AddLoft => "add_loft",
            Self::AddCoil => "add_coil",
            Self::AddRib => "add_rib",
            Self::AddThread => "add_thread",
            Self::AddSplitFace => "add_split_face",
            Self::AddDeleteFace => "add_delete_face",
            Self::AddEmboss => "add_emboss",
            Self::AddMoveFace => "add_move_face",
            Self::AddDraft => "add_draft",
            Self::AddReplaceFace => "add_replace_face",
            Self::AddSplit => "add_split",
            Self::AddScale => "add_scale",
            Self::AddDatumPoint => "add_datum_point",
            Self::AddCoordinateSystem => "add_coordinate_system",
            Self::AddMate => "add_mate",
            Self::AddHelix => "add_helix",
            Self::AddCurvePattern => "add_curve_pattern",
            Self::AddPointPattern => "add_point_pattern",
            Self::Import => "import",
            Self::SaveAs => "save_as",
            Self::EditFeature => "edit_feature",
            Self::RenameFeature => "rename_feature",
            Self::SuppressFeatures => "suppress_features",
            Self::MoveFeature => "move_feature",
            Self::DeleteFeatures => "delete_features",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::GetModel => "Model summary",
            Self::ListFeatures => "Feature tree",
            Self::ListParameters => "Parameters",
            Self::ListBodies => "Bodies",
            Self::ListFaces => "Faces of a body",
            Self::GetSketch => "Sketch",
            Self::RenderView => "Picture of the model",
            Self::SetParameter => "Set a parameter",
            Self::AddSketch => "New sketch",
            Self::AddSketchGeometry => "Draw in a sketch",
            Self::AddSketchConstraints => "Constrain a sketch",
            Self::RemoveSketchItems => "Remove from a sketch",
            Self::EditSketchConstraint => "Edit a constraint",
            Self::ModifySketch => "Change sketch geometry",
            Self::AddExtrusion => "Extrude",
            Self::AddHole => "Hole",
            Self::Undo => "Undo",
            Self::Redo => "Redo",
            Self::Save => "Save",
            Self::Export => "Export",
            Self::ListEdges => "Edges of a body",
            Self::Measure => "Measure",
            Self::AddRevolve => "Revolve",
            Self::AddFillet => "Fillet",
            Self::AddChamfer => "Chamfer",
            Self::AddShell => "Shell",
            Self::AddOffsetFace => "Offset face",
            Self::AddPrimitive => "Primitive",
            Self::AddMirror => "Mirror",
            Self::AddLinearPattern => "Linear pattern",
            Self::AddCircularPattern => "Circular pattern",
            Self::AddCombine => "Combine",
            Self::AddMove => "Move body",
            Self::AddDatumPlane => "Datum plane",
            Self::AddDatumAxis => "Datum axis",
            Self::AddSweep => "Sweep",
            Self::AddPipe => "Pipe",
            Self::AddLoft => "Loft",
            Self::AddCoil => "Coil",
            Self::AddRib => "Rib",
            Self::AddThread => "Thread",
            Self::AddSplitFace => "Split face",
            Self::AddDeleteFace => "Delete face",
            Self::AddEmboss => "Emboss or deboss",
            Self::AddMoveFace => "Move face",
            Self::AddDraft => "Draft",
            Self::AddReplaceFace => "Replace face",
            Self::AddSplit => "Split body",
            Self::AddScale => "Scale body",
            Self::AddDatumPoint => "Datum point",
            Self::AddCoordinateSystem => "Coordinate system",
            Self::AddMate => "Mate",
            Self::AddHelix => "Helix",
            Self::AddCurvePattern => "Curve pattern",
            Self::AddPointPattern => "Point pattern",
            Self::Import => "Import",
            Self::SaveAs => "Save as",
            Self::EditFeature => "Edit a feature",
            Self::RenameFeature => "Rename a feature",
            Self::SuppressFeatures => "Suppress features",
            Self::MoveFeature => "Reorder a feature",
            Self::DeleteFeatures => "Delete features",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::GetModel => {
                "Summarises the open model: its file, whether it has unsaved changes, what undo \
                 and redo would do, how many features, parameters and bodies it has and which \
                 features fail."
            }
            Self::ListFeatures => {
                "Lists the feature tree in order: each feature's id, name, kind and state (up to \
                 date, failed, waiting, outdated, suppressed, below the rollback bar), with the \
                 plain reason and remedy of a failure, and a sketch's degrees of freedom."
            }
            Self::ListParameters => {
                "Lists the model's parameters: id, name, expression, value with its unit, note \
                 and what uses each."
            }
            Self::ListBodies => {
                "Lists the bodies standing at the end of the tree with their exact volume (mm³), \
                 surface area (mm²), centre of mass and bounding box (mm). Reports progress and \
                 can be cancelled."
            }
            Self::ListFaces => {
                "Lists the faces of one body with a stable key (pass it to add_sketch), a \
                 description in words, the kind of surface, a flat face's outward normal and \
                 point, a round face's radius and the area. With before, the faces as that feature received the body."
            }
            Self::GetSketch => {
                "Describes one sketch: its plane in model coordinates, what it lies on, its \
                 entities with positions in sketch coordinates (mm), its constraints and \
                 dimensions, the degrees of freedom left and any conflicting or redundant \
                 constraints named."
            }
            Self::RenderView => {
                "Draws the model as a PNG from a standard view, fitted to the bodies, as the \
                 headless image export does. Needs a graphics adapter."
            }
            Self::SetParameter => {
                "Sets a parameter's expression, adding the parameter when no parameter has that \
                 name. Expressions take units (mm, cm, m, um, deg, rad) and other parameters, \
                 such as \"width / 2 + 3 mm\". One undo step."
            }
            Self::AddSketch => {
                "Adds an empty sketch at the end of the tree on a principal plane (XY, XZ or YZ), \
                 on a flat face of a body (a key from list_faces), on a datum plane (datum), or \
                 on a plane of a coordinate system (frame with plane). One undo step. Returns \
                 the sketch's plane: sketch x and y map to its x_axis and y_axis from its origin."
            }
            Self::AddSketchGeometry => {
                "Draws points, lines, polylines, rectangles, circles, arcs, tangent arcs, slots, \
                 arc slots, polygons, splines, conics, ellipses and elliptical arcs into a \
                 sketch, in sketch coordinates (mm). A tangent arc continues a line, arc or \
                 spline already in the sketch from the point ending it. A \
                 rectangle is four joined lines held horizontal and vertical; a polyline's lines \
                 share their corner points; a slot, arc slot and polygon come joined, tangent and \
                 equal as the app's tools make them (a polygon's corners lie on a construction \
                 circle, which a radius dimension can drive). Returns the ids of what was drawn. \
                 One undo step."
            }
            Self::AddSketchConstraints => {
                "Adds constraints and dimensions to a sketch, referring to entities by the ids \
                 add_sketch_geometry and get_sketch give (or origin, horizontal_axis, \
                 vertical_axis). Values are expressions with units and parameters. Constraints \
                 that conflict with the sketch are refused, naming what they conflict with. One \
                 undo step."
            }
            Self::RemoveSketchItems => {
                "Removes constraints and dimensions (by the ids get_sketch gives) and entities \
                 from a sketch. Removing an entity removes the curves built on a point and the \
                 constraints using it; the answer lists everything removed. The sketch's origin \
                 and axes stay. One undo step."
            }
            Self::ModifySketch => {
                "Changes a sketch's geometry as the app's sketch tools do, one operation a call: \
                 trim pieces back to the curves crossing them, extend an end to the next curve, \
                 offset a chain, mirror about a line, repeat in rows or around a point, round or \
                 cut corners, move, rotate or scale entities (solved like a drag, so \
                 constraints hold), project edges, faces, corners, other sketches' curves, \
                 axes, datums and the origin into the sketch (following their source), \
                 intersect a body, face or plane with the sketch plane, switch curves to \
                 construction geometry and back, split a curve at a point, break curves at \
                 their crossings, respace old fit-point splines, add a control point to a spline keeping its shape, draw a circle tangent to two \
                 or three curves, blend two curve ends with a spline, draw a spur gear, gear \
                 pair or sprocket, or copy geometry (with its constraints) within a sketch or \
                 from another. Lengths and angles are expressions with units and parameters. \
                 Returns the entities added and removed. One undo step."
            }
            Self::EditSketchConstraint => {
                "Changes a dimension's value (an expression with units and parameters, such as \
                 \"30 mm\" or \"width / 2\") and/or turns a constraint on or off; a dimension \
                 turned off is a reference, measured but no longer driving. One undo step."
            }
            Self::AddExtrusion => {
                "Extrudes every closed region of a sketch, or the regions chosen by a point inside each, or a thin wall along its curves. Ends at a distance, through all, the next face, a plane or a curved face, on one side or both, square to the sketch or along an axis. With no operation, it decides like \
                 the app: a sketch on a face cuts into that body, otherwise it adds to the body \
                 it reaches or makes a new body. With an operation (new_body, add, remove, \
                 intersect) it does that; a cut from a sketch on the cut body's face runs into \
                 it. One undo step; returns any feature the change made fail."
            }
            Self::AddHole => {
                "Drills a hole at every free point and circle centre of a sketch, square to the \
                 sketch plane, into the body the sketch lies on (or the given body). Depth is an \
                 expression or \"through all\". Plain, counterbored, countersunk or stepped, \
                 round or slot, flat or drill point, sized by the diameter given or by the \
                 circles drawn. One undo step."
            }
            Self::Undo => "Undoes the last change, whoever made it in this session.",
            Self::Redo => "Redoes the last change undone.",
            Self::Save => {
                "Saves the model to its file, atomically, keeping the version it replaces in \
                 the file's history."
            }
            Self::Export => {
                "Exports the bodies to a file in the model's folder or one the user allowed: \
                 .step or .stp, .stl, .3mf, .obj, .glb, or .png (a picture, which needs a \
                 graphics adapter). A relative path is taken from the model's folder."
            }
            Self::ListEdges => {
                "Lists the edges of one body with a stable key (pass it to add_fillet, add_chamfer, measure or as an axis), a description in words naming the faces it lies between, its kind (line, circle, arc, ellipse, curve), its length and a line's ends or a round edge's centre and radius. With before, the edges as that feature received the body."
            }
            Self::Measure => {
                "Measures the shortest distance between two things, each a point, a face or an edge of a body, with the closest points on each, and the angle between them when they have one (two planes, two lines, a line and a plane)."
            }
            Self::AddRevolve => {
                "Revolves every closed region of a sketch (or the regions chosen by a point inside each, or a thin wall along its curves) about an axis: a line of the sketch, its horizontal or vertical axis, or a model axis. A full turn unless an angle is given, or up to a plane. With no operation it decides like the app: it adds to the body it reaches or makes a new body. One undo step."
            }
            Self::AddFillet => {
                "Rounds edges of a body: edge keys from list_edges, every edge around faces from list_faces, or every edge of the body. One undo step; a radius too large for the edges fails the feature with the reason."
            }
            Self::AddChamfer => {
                "Bevels edges of a body, chosen as add_fillet chooses them, by one distance, two distances or a distance and an angle. One undo step."
            }
            Self::AddShell => {
                "Hollows a body out to a wall thickness, leaving the given faces open. One undo step."
            }
            Self::AddOffsetFace => {
                "Moves faces of a body along their normals by a distance, the faces around them following, so a body grows or shrinks there. One undo step."
            }
            Self::AddPrimitive => {
                "Adds a box, cylinder, sphere, torus, cone, wedge or prism on a plane or a flat face, sized by expressions. On a face it adds to that body unless an operation says otherwise; elsewhere it makes a new body. One undo step."
            }
            Self::AddMirror => {
                "Mirrors a body, or holes, cuts and bosses of one body, across a plane: a principal plane, a flat face or a datum plane. One undo step."
            }
            Self::AddLinearPattern => {
                "Repeats a body, or holes, cuts and bosses of one body, in a row along an axis, and optionally along a second axis as a grid. One undo step."
            }
            Self::AddCircularPattern => {
                "Repeats a body, or holes, cuts and bosses of one body, around an axis, spread evenly over a full turn or a given angle. One undo step."
            }
            Self::AddCombine => {
                "Joins bodies to a body, cuts them from it or keeps only what they share. One undo step."
            }
            Self::AddMove => {
                "Moves or turns a body, or a copy of it, by offsets along X, Y and Z and turns about its own centre or about an axis. One undo step."
            }
            Self::AddDatumPlane => {
                "Adds a datum plane for sketches, mirrors and primitives to use: offset from a plane or flat face and optionally turned about an axis, through three points, midway between two planes, through two lines, holding an axis and a point, square to an axis at a point, touching a curved face nearest a point, or square to an edge at a distance along it. One undo step."
            }
            Self::AddDatumAxis => {
                "Adds a datum axis along an edge, round face, sketch line or principal axis, where two planes meet, through two points, square to a plane at a point, or square to a curved face nearest a point. One undo step."
            }
            Self::AddDatumPoint => {
                "Adds a datum point at a point (origin, a corner, a round edge's or sphere's centre, a sketch point, another datum point) with an offset, where two lines cross, where an axis meets a plane, where three planes meet, at a distance along an edge, at an edge's middle or at a face's centre. One undo step."
            }
            Self::AddCoordinateSystem => {
                "Adds a coordinate system from an origin point, an axis for its X direction and a plane for its XY plane, either reversed; sketches, mates and other features can use its planes, axes and origin. One undo step."
            }
            Self::AddMate => {
                "Moves a body by its geometry onto another body, a plane, an axis or a point, and keeps it there when that changes: a flat face flush with a plane (at a distance), an axis concentric with another, both at once, a round face resting on a plane, a flat face resting on a round face, a point on a point or plane, or a face or axis turned to an angle. One undo step."
            }
            Self::AddHelix => {
                "Adds a helix about an axis, sized by its diameter and two of turns, pitch and height, optionally tapered, for pipes and sweeps to follow. One undo step."
            }
            Self::AddCurvePattern => {
                "Repeats a body, or holes, cuts and bosses of one body, along the curves of a sketch joined end to end, spread evenly or a spacing apart, keeping or following the curve's turn. One undo step."
            }
            Self::AddPointPattern => {
                "Repeats a body, or holes, cuts and bosses of one body, at each lone point of a sketch, each copy moved from a base point. One undo step."
            }
            Self::Import => {
                "Imports a file from the model's folder or one the user allowed: the bodies of a STEP model or an STL, OBJ or 3MF mesh, or a DXF or SVG drawing into a new sketch on a plane or into an existing sketch. A relative path is taken from the model's folder. One undo step."
            }
            Self::SaveAs => {
                "Saves the model under a new name ending in .caditor, in the model's folder or one the user allowed, and goes on working on that file; it never replaces an existing file."
            }
            Self::AddSweep => {
                "Sweeps every closed region of a sketch along a path: lines, arcs and curves of another sketch, edges of a body, or a helix. With no operation it adds to the body it reaches or makes a new body. One undo step."
            }
            Self::AddPipe => {
                "Runs a round, square or triangular section of a size along a path (sketch curves, body edges or a helix), bending sharp corners by a bend radius, solid or hollow with a wall. One undo step."
            }
            Self::AddLoft => {
                "Lofts a solid through the closed regions of two or more sketches, in order, smoothly or ruled. One undo step."
            }
            Self::AddCoil => {
                "Winds a round or square section along a helix about an axis, sized by its diameter and two of turns, pitch and height, optionally tapered. One undo step."
            }
            Self::AddRib => {
                "Thickens an open line or arc chain of a sketch into a rib or web joining a body, grown until it meets the body. One undo step."
            }
            Self::AddThread => {
                "Threads a round face of a body (a bore, shaft or boss) to a standard size, modelled or cosmetic, over its whole length or a length. One undo step."
            }
            Self::AddSplitFace => {
                "Splits faces of a body along a plane, a sketch's curves or another body, so each piece can be picked on its own. One undo step."
            }
            Self::AddDeleteFace => {
                "Deletes faces of a body and heals the gap the surrounding faces leave. One undo step."
            }
            Self::AddEmboss => {
                "Raises the closed outlines or text of a sketch (all of them, or the regions chosen by a point inside each) from faces of a body, or recesses them into it, by a depth: flat faces of one plane take the sketch parallel to them, faces of one cylinder or cone take it wrapped round them. One undo step."
            }
            Self::AddMoveFace => {
                "Shifts faces of a body along a direction and tilts them about an axis, the faces around them extending or trimming to follow, as on an imported body with no history. One undo step."
            }
            Self::AddDraft => {
                "Tilts flat faces of a body about the line where each meets the neutral plane, and turns round faces (cylinders and cones) about the pull direction into cones hinged on the circle where each meets it, so each makes the draft angle with the pull direction (the neutral plane's normal, reversed by flipped), the faces around them following; beyond splits the faces at the neutral plane and drafts the part beyond it at an angle of its own. One undo step."
            }
            Self::AddReplaceFace => {
                "Replaces flat faces of a body with a target plane (a principal, datum or coordinate system plane, or a flat face of any body), each face turned and moved onto it keeping the side it faces, the faces around them extending or trimming to meet it, as on an imported body with no history. One undo step."
            }
            Self::AddSplit => {
                "Splits a body in two along a plane, a sketch's curves or another body; flipped swaps which side is kept first. One undo step."
            }
            Self::AddScale => "Scales a body by a factor about a centre point. One undo step.",
            Self::EditFeature => {
                "Changes an existing feature, as one undo step, with the arguments of the tool that adds its kind (add_extrusion for an extrusion, add_fillet for a fillet, and so on): what is given replaces that part, the rest stays as it is, and name renames it. Faces and edges of the feature's own body are named as that feature received it (list_faces and list_edges with before), so an edge a fillet rounded away can be named again. Sketches change through the sketch tools."
            }
            Self::RenameFeature => "Renames a feature. One undo step.",
            Self::SuppressFeatures => {
                "Suppresses features, leaving them out when the model is computed, or brings them back with suppressed false. One undo step."
            }
            Self::MoveFeature => {
                "Moves a feature before or after another in the tree; refused when it would come before what it uses or after what uses it. One undo step."
            }
            Self::DeleteFeatures => {
                "Deletes features. When other features use them, dependents says whether those go too or stay, failing; without it the call is refused naming them. One undo step."
            }
        }
    }

    fn effect(self) -> Effect {
        match self {
            Self::GetModel
            | Self::ListFeatures
            | Self::ListParameters
            | Self::ListBodies
            | Self::ListFaces
            | Self::GetSketch
            | Self::RenderView
            | Self::ListEdges
            | Self::Measure => Effect::Reads,
            Self::AddSketch
            | Self::AddSketchGeometry
            | Self::AddSketchConstraints
            | Self::AddExtrusion
            | Self::AddHole
            | Self::AddRevolve
            | Self::AddFillet
            | Self::AddChamfer
            | Self::AddShell
            | Self::AddOffsetFace
            | Self::AddPrimitive
            | Self::AddMirror
            | Self::AddLinearPattern
            | Self::AddCircularPattern
            | Self::AddCombine
            | Self::AddMove
            | Self::AddDatumPlane
            | Self::AddDatumAxis
            | Self::AddSweep
            | Self::AddPipe
            | Self::AddLoft
            | Self::AddCoil
            | Self::AddRib
            | Self::AddThread
            | Self::AddSplitFace
            | Self::AddDeleteFace
            | Self::AddEmboss
            | Self::AddMoveFace
            | Self::AddDraft
            | Self::AddReplaceFace
            | Self::AddSplit
            | Self::AddScale
            | Self::AddDatumPoint
            | Self::AddCoordinateSystem
            | Self::AddMate
            | Self::AddHelix
            | Self::AddCurvePattern
            | Self::AddPointPattern
            | Self::Import => Effect::Adds,
            Self::SetParameter
            | Self::RemoveSketchItems
            | Self::EditSketchConstraint
            | Self::ModifySketch
            | Self::Undo
            | Self::Redo
            | Self::RenameFeature
            | Self::SuppressFeatures
            | Self::MoveFeature
            | Self::DeleteFeatures
            | Self::EditFeature => Effect::Changes,
            Self::Save | Self::Export | Self::SaveAs => Effect::Writes,
        }
    }

    fn input_schema(self) -> Value {
        let nothing = json!({ "type": "object", "properties": {}, "additionalProperties": false });
        match self {
            Self::GetModel
            | Self::ListFeatures
            | Self::ListParameters
            | Self::ListBodies
            | Self::Undo
            | Self::Redo
            | Self::Save => nothing,
            Self::ListFaces => object(
                json!({
                    "body": reference("The body, by id or name from list_bodies"),
                    "before": reference("A feature by id or name: list the body as that feature received it, which is what edit_feature resolves the faces and edges it is given on")
                }),
                &["body"],
            ),
            Self::GetSketch => object(
                json!({ "sketch": reference("The sketch, by feature id or name") }),
                &["sketch"],
            ),
            Self::RenderView => object(
                json!({
                    "view": {
                        "type": "string",
                        "enum": ["home", "isometric", "front", "top", "right", "back", "bottom", "left"],
                        "description": "Where the picture looks from; home is the model's home view, else isometric"
                    },
                    "width": { "type": "integer", "minimum": reading::SMALLEST_SIDE, "maximum": reading::LARGEST_SIDE },
                    "height": { "type": "integer", "minimum": reading::SMALLEST_SIDE, "maximum": reading::LARGEST_SIDE }
                }),
                &[],
            ),
            Self::SetParameter => object(
                json!({
                    "name": { "type": "string", "description": "The parameter's name: letters, digits and _, starting with a letter" },
                    "expression": { "type": "string", "description": "Such as \"80 mm\" or \"width - 2 * margin\"" }
                }),
                &["name", "expression"],
            ),
            Self::AddSketch => object(
                json!({
                    "plane": { "type": "string", "enum": ["XY", "XZ", "YZ"], "description": "A principal plane, or with frame the plane of that coordinate system" },
                    "body": reference("With face: the body the face belongs to"),
                    "face": { "type": "string", "description": "A flat face's key from list_faces" },
                    "datum": reference("A datum plane, by feature id or name from list_features"),
                    "frame": reference("A coordinate system, by feature id or name from list_features; with plane"),
                    "name": { "type": "string" }
                }),
                &[],
            ),
            Self::AddSketchGeometry => object(
                json!({
                    "sketch": reference("The sketch, by feature id or name"),
                    "construction": { "type": "boolean", "description": "Draw lines and circles as construction geometry, which makes no regions" },
                    "items": {
                        "type": "array",
                        "minItems": 1,
                        "maxItems": writing::MOST_ITEMS,
                        "items": {
                            "type": "object",
                            "properties": {
                                "kind": { "type": "string", "enum": ["point", "line", "polyline", "rectangle", "circle", "arc", "tangent_arc", "slot", "arc_slot", "polygon", "spline", "conic", "ellipse", "elliptical_arc"] },
                                "at": point("point: where it is"),
                                "from": point("line: its start; slot: the first end's centre; arc_slot: where the centre line starts; polygon: one end of a side, instead of center"),
                                "to": point("line: its end; slot: the second end's centre; arc_slot: where the centre line ends, counter-clockwise from from; polygon: the side's other end, the polygon lying to the left of from to to; tangent_arc: where it ends"),
                                "from_point": entity(),
                                "after_item": { "type": "integer", "minimum": 0, "description": "tangent_arc: instead of from_point, the index in items of an earlier line, arc, polyline, spline or tangent arc of this call, whose end the arc leaves along its tangent" },
                                "points": { "type": "array", "items": point("A corner"), "description": "polyline: its corners in order; spline: its points in order" },
                                "closed": { "type": "boolean", "description": "polyline: join the last corner to the first; spline: close it into a loop (three points at least)" },
                                "corner": point("rectangle: one corner; polygon: a vertex, which fixes its size and turn"),
                                "opposite": point("rectangle: the opposite corner"),
                                "center": point("circle, arc, arc_slot, polygon, ellipse, elliptical_arc: the centre"),
                                "radius": { "type": "number", "exclusiveMinimum": 0 },
                                "start": point("arc: where it starts; elliptical_arc: its direction from the centre sets the start; conic: where it starts"),
                                "end": point("arc: where it ends; with center only its direction from the centre counts, as for elliptical_arc; conic: where it ends"),
                                "apex": point("conic: where the tangents at its ends meet"),
                                "rho": { "type": ["number", "string"], "description": "conic: its fullness from 0.01 to 0.99, 0.5 (a parabola) by default; lower is flatter (an ellipse), higher sharper (a hyperbola); an expression using parameters, such as \"fullness\", holds it by a Rho dimension" },
                                "through": point("arc: a point it passes through, instead of center"),
                                "clockwise": { "type": "boolean", "description": "arc with center, elliptical_arc: run clockwise from start to end instead of counter-clockwise" },
                                "width": { "type": "number", "exclusiveMinimum": 0, "description": "slot, arc_slot: the full width across" },
                                "sides": { "type": "integer", "minimum": 3, "maximum": 64, "description": "polygon: the number of sides, 6 by default" },
                                "side_middle": point("polygon: the middle of its first side, instead of corner"),
                                "form": { "type": "string", "enum": ["fit", "control"], "description": "spline: through its points (fit, the default) or shaped by them as control points" },
                                "major_end": point("ellipse, elliptical_arc: the end of its major axis"),
                                "minor_radius": { "type": "number", "exclusiveMinimum": 0, "description": "ellipse, elliptical_arc: half its width across the minor axis" }
                            },
                            "required": ["kind"]
                        }
                    }
                }),
                &["sketch", "items"],
            ),
            Self::AddSketchConstraints => object(
                json!({
                    "sketch": reference("The sketch, by feature id or name"),
                    "constraints": {
                        "type": "array",
                        "minItems": 1,
                        "maxItems": writing::MOST_ITEMS,
                        "items": {
                            "type": "object",
                            "description": "coincident, parallel, perpendicular, equal, tangent, concentric, collinear take a and b; horizontal and vertical take line; midpoint takes point and curve; symmetric takes a, b and about; fix takes point; distance, horizontal_distance, vertical_distance and angle take from, to and value; radius and diameter take curve and value",
                            "properties": {
                                "kind": { "type": "string", "enum": [
                                    "coincident", "horizontal", "vertical", "parallel", "perpendicular", "equal",
                                    "tangent", "concentric", "collinear", "midpoint", "symmetric", "fix",
                                    "distance", "horizontal_distance", "vertical_distance", "radius", "diameter", "angle"
                                ] },
                                "a": entity(), "b": entity(), "line": entity(), "point": entity(),
                                "curve": entity(), "about": entity(), "from": entity(), "to": entity(),
                                "value": { "type": "string", "description": "An expression such as \"80 mm\", \"length\" or \"30 deg\"" }
                            },
                            "required": ["kind"]
                        }
                    }
                }),
                &["sketch", "constraints"],
            ),
            Self::RemoveSketchItems => object(
                json!({
                    "sketch": reference("The sketch, by feature id or name"),
                    "constraints": { "type": "array", "items": { "type": "integer" }, "maxItems": writing::MOST_ITEMS, "description": "Constraint and dimension ids from get_sketch" },
                    "entities": { "type": "array", "items": entity(), "maxItems": writing::MOST_ITEMS, "description": "Entity ids from add_sketch_geometry and get_sketch" }
                }),
                &["sketch"],
            ),
            Self::ModifySketch => sketch_schemas::modify_sketch(),
            Self::EditSketchConstraint => object(
                json!({
                    "sketch": reference("The sketch, by feature id or name"),
                    "constraint": { "type": "integer", "description": "The constraint or dimension id from get_sketch" },
                    "value": { "type": "string", "description": "A dimension's new value as an expression such as \"80 mm\", \"30 deg\" or \"length / 2\"" },
                    "active": { "type": "boolean", "description": "false turns the constraint off (a dimension becomes a reference), true turns it on" }
                }),
                &["sketch", "constraint"],
            ),
            Self::AddExtrusion => sweep_schemas::extrusion(),
            Self::AddHole => sweep_schemas::hole(),
            Self::Export => object(
                json!({
                    "file": { "type": "string", "description": "The file to write, in the model's folder or an allowed one" },
                    "resolution": { "type": "string", "enum": ["coarse", "standard", "fine"], "description": "Mesh quality of STL, 3MF, OBJ and glTF" },
                    "width": { "type": "integer", "minimum": reading::SMALLEST_SIDE, "maximum": reading::LARGEST_SIDE },
                    "height": { "type": "integer", "minimum": reading::SMALLEST_SIDE, "maximum": reading::LARGEST_SIDE }
                }),
                &["file"],
            ),
            Self::ListEdges => feature_schemas::list_edges(),
            Self::Measure => feature_schemas::measure(),
            Self::AddRevolve => feature_schemas::revolve(),
            Self::AddFillet => feature_schemas::fillet(),
            Self::AddChamfer => feature_schemas::chamfer(),
            Self::AddShell => feature_schemas::shell(),
            Self::AddOffsetFace => feature_schemas::offset_face(),
            Self::AddPrimitive => feature_schemas::primitive(),
            Self::AddMirror => feature_schemas::mirror(),
            Self::AddLinearPattern => feature_schemas::linear_pattern(),
            Self::AddCircularPattern => feature_schemas::circular_pattern(),
            Self::AddCombine => feature_schemas::combine(),
            Self::AddMove => feature_schemas::move_body(),
            Self::AddDatumPlane => datum_schemas::datum_plane(),
            Self::AddDatumAxis => datum_schemas::datum_axis(),
            Self::AddSweep => sweep_schemas::sweep(),
            Self::AddPipe => sweep_schemas::pipe(),
            Self::AddLoft => sweep_schemas::loft(),
            Self::AddCoil => sweep_schemas::coil(),
            Self::AddRib => sweep_schemas::rib(),
            Self::AddThread => sweep_schemas::thread(),
            Self::AddSplitFace => sweep_schemas::split_face(),
            Self::AddDeleteFace => sweep_schemas::delete_face(),
            Self::AddEmboss => sweep_schemas::emboss(),
            Self::AddMoveFace => sweep_schemas::move_face(),
            Self::AddDraft => sweep_schemas::draft(),
            Self::AddReplaceFace => sweep_schemas::replace_face(),
            Self::AddSplit => sweep_schemas::split(),
            Self::AddScale => sweep_schemas::scale(),
            Self::AddDatumPoint => datum_schemas::datum_point(),
            Self::AddCoordinateSystem => datum_schemas::coordinate_system(),
            Self::AddMate => datum_schemas::mate(),
            Self::AddHelix => datum_schemas::helix(),
            Self::AddCurvePattern => datum_schemas::curve_pattern(),
            Self::AddPointPattern => datum_schemas::point_pattern(),
            Self::Import => datum_schemas::import(),
            Self::SaveAs => datum_schemas::save_as(),
            Self::EditFeature => sweep_schemas::edit_feature(),
            Self::RenameFeature => feature_schemas::rename_feature(),
            Self::SuppressFeatures => feature_schemas::suppress_features(),
            Self::MoveFeature => feature_schemas::move_feature(),
            Self::DeleteFeatures => feature_schemas::delete_features(),
        }
    }

    pub fn listing(self) -> Value {
        let effect = self.effect();
        json!({
            "name": self.name(),
            "title": self.title(),
            "description": self.description(),
            "inputSchema": self.input_schema(),
            "annotations": {
                "title": self.title(),
                "readOnlyHint": effect == Effect::Reads,
                "destructiveHint": effect == Effect::Changes,
                "idempotentHint": matches!(effect, Effect::Reads | Effect::Writes),
                "openWorldHint": false
            }
        })
    }

    pub fn decode(self, arguments: Value) -> Result<Call, ToolError> {
        let arguments = match arguments {
            Value::Null => Value::Object(Map::new()),
            other => other,
        };
        Ok(match self {
            Self::GetModel => Call::GetModel(self.arguments(arguments)?),
            Self::ListFeatures => Call::ListFeatures(self.arguments(arguments)?),
            Self::ListParameters => Call::ListParameters(self.arguments(arguments)?),
            Self::ListBodies => Call::ListBodies(self.arguments(arguments)?),
            Self::ListFaces => Call::ListFaces(self.arguments(arguments)?),
            Self::GetSketch => Call::GetSketch(self.arguments(arguments)?),
            Self::RenderView => Call::RenderView(self.arguments(arguments)?),
            Self::SetParameter => Call::SetParameter(self.arguments(arguments)?),
            Self::AddSketch => Call::AddSketch(self.arguments(arguments)?),
            Self::AddSketchGeometry => Call::AddSketchGeometry(self.arguments(arguments)?),
            Self::AddSketchConstraints => Call::AddSketchConstraints(self.arguments(arguments)?),
            Self::RemoveSketchItems => Call::RemoveSketchItems(self.arguments(arguments)?),
            Self::EditSketchConstraint => Call::EditSketchConstraint(self.arguments(arguments)?),
            Self::ModifySketch => Call::ModifySketch(self.arguments(arguments)?),
            Self::AddExtrusion => Call::AddExtrusion(self.arguments(arguments)?),
            Self::AddHole => Call::AddHole(self.arguments(arguments)?),
            Self::Undo => Call::Undo(self.arguments(arguments)?),
            Self::Redo => Call::Redo(self.arguments(arguments)?),
            Self::Save => Call::Save(self.arguments(arguments)?),
            Self::Export => Call::Export(self.arguments(arguments)?),
            Self::ListEdges => Call::ListEdges(self.arguments(arguments)?),
            Self::Measure => Call::Measure(self.arguments(arguments)?),
            Self::AddRevolve => Call::AddRevolve(self.arguments(arguments)?),
            Self::AddFillet => Call::AddFillet(self.arguments(arguments)?),
            Self::AddChamfer => Call::AddChamfer(self.arguments(arguments)?),
            Self::AddShell => Call::AddShell(self.arguments(arguments)?),
            Self::AddOffsetFace => Call::AddOffsetFace(self.arguments(arguments)?),
            Self::AddPrimitive => Call::AddPrimitive(self.arguments(arguments)?),
            Self::AddMirror => Call::AddMirror(self.arguments(arguments)?),
            Self::AddLinearPattern => Call::AddLinearPattern(self.arguments(arguments)?),
            Self::AddCircularPattern => Call::AddCircularPattern(self.arguments(arguments)?),
            Self::AddCombine => Call::AddCombine(self.arguments(arguments)?),
            Self::AddMove => Call::AddMove(self.arguments(arguments)?),
            Self::AddDatumPlane => Call::AddDatumPlane(self.arguments(arguments)?),
            Self::AddDatumAxis => Call::AddDatumAxis(self.arguments(arguments)?),
            Self::AddSweep => Call::AddSweep(self.arguments(arguments)?),
            Self::AddPipe => Call::AddPipe(self.arguments(arguments)?),
            Self::AddLoft => Call::AddLoft(self.arguments(arguments)?),
            Self::AddCoil => Call::AddCoil(self.arguments(arguments)?),
            Self::AddRib => Call::AddRib(self.arguments(arguments)?),
            Self::AddThread => Call::AddThread(self.arguments(arguments)?),
            Self::AddSplitFace => Call::AddSplitFace(self.arguments(arguments)?),
            Self::AddDeleteFace => Call::AddDeleteFace(self.arguments(arguments)?),
            Self::AddEmboss => Call::AddEmboss(self.arguments(arguments)?),
            Self::AddMoveFace => Call::AddMoveFace(self.arguments(arguments)?),
            Self::AddDraft => Call::AddDraft(self.arguments(arguments)?),
            Self::AddReplaceFace => Call::AddReplaceFace(self.arguments(arguments)?),
            Self::AddSplit => Call::AddSplit(self.arguments(arguments)?),
            Self::AddScale => Call::AddScale(self.arguments(arguments)?),
            Self::AddDatumPoint => Call::AddDatumPoint(self.arguments(arguments)?),
            Self::AddCoordinateSystem => Call::AddCoordinateSystem(self.arguments(arguments)?),
            Self::AddMate => Call::AddMate(self.arguments(arguments)?),
            Self::AddHelix => Call::AddHelix(self.arguments(arguments)?),
            Self::AddCurvePattern => Call::AddCurvePattern(self.arguments(arguments)?),
            Self::AddPointPattern => Call::AddPointPattern(self.arguments(arguments)?),
            Self::Import => Call::Import(self.arguments(arguments)?),
            Self::SaveAs => Call::SaveAs(self.arguments(arguments)?),
            Self::EditFeature => Call::EditFeature(self.arguments(arguments)?),
            Self::RenameFeature => Call::RenameFeature(self.arguments(arguments)?),
            Self::SuppressFeatures => Call::SuppressFeatures(self.arguments(arguments)?),
            Self::MoveFeature => Call::MoveFeature(self.arguments(arguments)?),
            Self::DeleteFeatures => Call::DeleteFeatures(self.arguments(arguments)?),
        })
    }

    fn arguments<T: DeserializeOwned>(self, arguments: Value) -> Result<T, ToolError> {
        serde_json::from_value(arguments).map_err(|source| ToolError::Arguments {
            tool: self.name(),
            source,
        })
    }
}

pub enum Call {
    GetModel(reading::Nothing),
    ListFeatures(reading::Nothing),
    ListParameters(reading::Nothing),
    ListBodies(reading::Nothing),
    ListFaces(reading::OfBody),
    GetSketch(reading::OfSketch),
    RenderView(reading::Picture),
    SetParameter(writing::ParameterValue),
    AddSketch(writing::NewSketch),
    AddSketchGeometry(writing::Drawing),
    AddSketchConstraints(writing::Constraining),
    RemoveSketchItems(sketching::Removing),
    EditSketchConstraint(sketching::ConstraintEdit),
    ModifySketch(sketch_edits::Modification),
    AddExtrusion(solids::Extrusion),
    AddHole(solids::Drilling),
    Undo(reading::Nothing),
    Redo(reading::Nothing),
    Save(reading::Nothing),
    Export(files::Exporting),
    ListEdges(reading::OfBody),
    Measure(edges::Between),
    AddRevolve(features::Revolution),
    AddFillet(features::Filleting),
    AddChamfer(features::Chamfering),
    AddShell(features::Shelling),
    AddOffsetFace(features::FaceOffset),
    AddPrimitive(features::NewPrimitive),
    AddMirror(features::Mirroring),
    AddLinearPattern(features::LinearPatterning),
    AddCircularPattern(features::CircularPatterning),
    AddCombine(features::Combining),
    AddMove(features::Moving),
    AddDatumPlane(datums::DatumPlaning),
    AddDatumAxis(datums::DatumAxising),
    AddSweep(sweeps::Sweeping),
    AddPipe(sweeps::Piping),
    AddLoft(sweeps::Lofting),
    AddCoil(sweeps::Coiling),
    AddRib(sweeps::Ribbing),
    AddThread(sweeps::Threading),
    AddSplitFace(reshaping::FaceSplitting),
    AddDeleteFace(reshaping::FaceDeleting),
    AddEmboss(reshaping::Embossing),
    AddMoveFace(reshaping::FaceMoving),
    AddDraft(reshaping::FaceDrafting),
    AddReplaceFace(reshaping::FaceReplacing),
    AddSplit(reshaping::Splitting),
    AddScale(reshaping::Scaling),
    AddDatumPoint(datums::DatumPointing),
    AddCoordinateSystem(datums::Framing),
    AddMate(mates::Mating),
    AddHelix(curves::Helixing),
    AddCurvePattern(curves::CurvePatterning),
    AddPointPattern(curves::PointPatterning),
    Import(files::Importing),
    SaveAs(files::SavingAs),
    EditFeature(edits::Editing),
    RenameFeature(tree_edits::Renaming),
    SuppressFeatures(tree_edits::Suppressing),
    MoveFeature(tree_edits::Reordering),
    DeleteFeatures(tree_edits::Deleting),
}

impl Call {
    pub fn run(self, session: &mut Session, work: &Work<'_>) -> Result<Answer, ToolError> {
        match self {
            Self::GetModel(_) => reading::model(session, work).map(Answer::of),
            Self::ListFeatures(_) => reading::features(session, work).map(Answer::of),
            Self::ListParameters(_) => Ok(Answer::of(reading::parameters(session))),
            Self::ListBodies(_) => reading::bodies(session, work).map(Answer::of),
            Self::ListFaces(of) => reading::faces(session, work, &of).map(Answer::of),
            Self::GetSketch(of) => reading::sketch(session, work, &of).map(Answer::of),
            Self::RenderView(picture) => reading::render(session, work, &picture),
            Self::SetParameter(value) => writing::set_parameter(session, work, value),
            Self::AddSketch(sketch) => writing::add_sketch(session, work, sketch),
            Self::AddSketchGeometry(drawing) => writing::draw(session, work, drawing),
            Self::AddSketchConstraints(constraining) => {
                writing::constrain(session, work, constraining)
            }
            Self::RemoveSketchItems(removing) => sketching::remove(session, work, removing),
            Self::EditSketchConstraint(edit) => sketching::edit_constraint(session, work, edit),
            Self::ModifySketch(modification) => sketch_edits::modify(session, work, modification),
            Self::AddExtrusion(extrusion) => solids::extrude(session, work, extrusion),
            Self::AddHole(drilling) => solids::drill(session, work, drilling),
            Self::Undo(_) => writing::undo(session, work),
            Self::Redo(_) => writing::redo(session, work),
            Self::Save(_) => files::save(session).map(Answer::of),
            Self::Export(exporting) => files::export(session, work, &exporting).map(Answer::of),
            Self::ListEdges(of) => edges::edges(session, work, &of).map(Answer::of),
            Self::Measure(between) => edges::measure(session, work, &between).map(Answer::of),
            Self::AddRevolve(revolution) => features::revolve(session, work, revolution),
            Self::AddFillet(filleting) => features::fillet(session, work, filleting),
            Self::AddChamfer(chamfering) => features::chamfer(session, work, chamfering),
            Self::AddShell(shelling) => features::shell(session, work, shelling),
            Self::AddOffsetFace(offset) => features::offset_face(session, work, offset),
            Self::AddPrimitive(primitive) => features::primitive(session, work, primitive),
            Self::AddMirror(mirroring) => features::mirror(session, work, mirroring),
            Self::AddLinearPattern(patterning) => {
                features::linear_pattern(session, work, patterning)
            }
            Self::AddCircularPattern(patterning) => {
                features::circular_pattern(session, work, patterning)
            }
            Self::AddCombine(combining) => features::combine(session, work, combining),
            Self::AddMove(moving) => features::move_body(session, work, moving),
            Self::AddDatumPlane(planing) => datums::datum_plane(session, work, planing),
            Self::AddDatumAxis(axising) => datums::datum_axis(session, work, axising),
            Self::AddSweep(sweeping) => sweeps::sweep(session, work, sweeping),
            Self::AddPipe(piping) => sweeps::pipe(session, work, piping),
            Self::AddLoft(lofting) => sweeps::loft(session, work, lofting),
            Self::AddCoil(coiling) => sweeps::coil(session, work, coiling),
            Self::AddRib(ribbing) => sweeps::rib(session, work, ribbing),
            Self::AddThread(threading) => sweeps::thread(session, work, threading),
            Self::AddSplitFace(splitting) => reshaping::split_face(session, work, splitting),
            Self::AddDeleteFace(deleting) => reshaping::delete_face(session, work, deleting),
            Self::AddEmboss(embossing) => reshaping::emboss(session, work, embossing),
            Self::AddMoveFace(moving) => reshaping::move_face(session, work, moving),
            Self::AddDraft(drafting) => reshaping::draft(session, work, drafting),
            Self::AddReplaceFace(replacing) => reshaping::replace_face(session, work, replacing),
            Self::AddSplit(splitting) => reshaping::split(session, work, splitting),
            Self::AddScale(scaling) => reshaping::scale(session, work, scaling),
            Self::AddDatumPoint(pointing) => datums::datum_point(session, work, pointing),
            Self::AddCoordinateSystem(framing) => datums::coordinate_system(session, work, framing),
            Self::AddMate(mating) => mates::mate(session, work, mating),
            Self::AddHelix(helixing) => curves::helix(session, work, helixing),
            Self::AddCurvePattern(patterning) => curves::curve_pattern(session, work, patterning),
            Self::AddPointPattern(patterning) => curves::point_pattern(session, work, patterning),
            Self::Import(importing) => files::import_file(session, work, &importing),
            Self::SaveAs(saving) => files::save_as(session, &saving).map(Answer::of),
            Self::EditFeature(editing) => edits::edit(session, work, editing),
            Self::RenameFeature(renaming) => tree_edits::rename(session, work, renaming),
            Self::SuppressFeatures(suppressing) => tree_edits::suppress(session, work, suppressing),
            Self::MoveFeature(reordering) => tree_edits::reorder(session, work, reordering),
            Self::DeleteFeatures(deleting) => tree_edits::delete(session, work, deleting),
        }
    }
}

pub(super) fn object(properties: Value, required: &[&str]) -> Value {
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false
    })
}

pub(super) fn reference(description: &str) -> Value {
    json!({ "type": ["integer", "string"], "description": description })
}

pub(super) fn point(description: &str) -> Value {
    json!({
        "type": "array",
        "items": { "type": "number" },
        "minItems": 2,
        "maxItems": 2,
        "description": description
    })
}

pub(super) fn entity() -> Value {
    json!({
        "type": ["integer", "string"],
        "description": "An entity id, or origin, horizontal_axis or vertical_axis"
    })
}
