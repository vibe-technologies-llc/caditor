use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};

use crate::{
    mcp::{
        edges::{self, MeasuringError},
        feature_schemas,
        features::{self, FeatureError},
        files::{self, FileError},
        places::PlaceError,
        reading::{self, RenderError},
        references::ReferenceError,
        session::{Cancelled, ChangeError, Session, Work},
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
        reason: String,
    },
    #[error(transparent)]
    Change(#[from] ChangeError),
    #[error(transparent)]
    Sketching(#[from] SketchingError),
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
                 point, a round face's radius and the area."
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
                "Adds an empty sketch on a principal plane (XY, XZ or YZ) or on a flat face of a \
                 body (a key from list_faces), at the end of the tree. One undo step. Returns the \
                 sketch's plane: sketch x and y map to its x_axis and y_axis from its origin."
            }
            Self::AddSketchGeometry => {
                "Draws points, lines, polylines, rectangles and circles into a sketch, in sketch \
                 coordinates (mm). A rectangle is four joined lines held horizontal and vertical; \
                 a polyline's lines share their corner points. Returns the ids of what was drawn. \
                 One undo step."
            }
            Self::AddSketchConstraints => {
                "Adds constraints and dimensions to a sketch, referring to entities by the ids \
                 add_sketch_geometry and get_sketch give (or origin, horizontal_axis, \
                 vertical_axis). Values are expressions with units and parameters. Constraints \
                 that conflict with the sketch are refused, naming what they conflict with. One \
                 undo step."
            }
            Self::AddExtrusion => {
                "Extrudes every closed region of a sketch. With no operation, it decides like \
                 the app: a sketch on a face cuts into that body, otherwise it adds to the body \
                 it reaches or makes a new body. With an operation (new_body, add, remove, \
                 intersect) it does that; a cut from a sketch on the cut body's face runs into \
                 it. One undo step; returns any feature the change made fail."
            }
            Self::AddHole => {
                "Drills a hole at every free point and circle centre of a sketch, square to the \
                 sketch plane, into the body the sketch lies on (or the given body). Depth is an \
                 expression or \"through all\". One undo step."
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
                "Lists the edges of one body with a stable key (pass it to add_fillet, add_chamfer, measure or as an axis), a description in words naming the faces it lies between, its kind (line, circle, arc, ellipse, curve), its length and a line's ends or a round edge's centre and radius."
            }
            Self::Measure => {
                "Measures the shortest distance between two things, each a point, a face or an edge of a body, with the closest points on each, and the angle between them when they have one (two planes, two lines, a line and a plane)."
            }
            Self::AddRevolve => {
                "Revolves every closed region of a sketch about an axis: a line of the sketch, its horizontal or vertical axis, or a model axis. A full turn unless an angle is given. With no operation it decides like the app: it adds to the body it reaches or makes a new body. One undo step."
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
                "Adds a datum plane offset from a plane or flat face and optionally turned about an axis, for sketches, mirrors and primitives to use. One undo step."
            }
            Self::AddDatumAxis => {
                "Adds a datum axis along an edge, round face, sketch line or principal axis, or where two planes meet. One undo step."
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
            | Self::AddDatumAxis => Effect::Adds,
            Self::SetParameter
            | Self::Undo
            | Self::Redo
            | Self::RenameFeature
            | Self::SuppressFeatures
            | Self::MoveFeature
            | Self::DeleteFeatures => Effect::Changes,
            Self::Save | Self::Export => Effect::Writes,
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
                json!({ "body": reference("The body, by id or name from list_bodies") }),
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
                    "plane": { "type": "string", "enum": ["XY", "XZ", "YZ"] },
                    "body": reference("With face: the body the face belongs to"),
                    "face": { "type": "string", "description": "A flat face's key from list_faces" },
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
                                "kind": { "type": "string", "enum": ["point", "line", "polyline", "rectangle", "circle"] },
                                "at": point("point: where it is"),
                                "from": point("line: its start"),
                                "to": point("line: its end"),
                                "points": { "type": "array", "items": point("A corner"), "description": "polyline: its corners in order" },
                                "closed": { "type": "boolean", "description": "polyline: join the last corner to the first" },
                                "corner": point("rectangle: one corner"),
                                "opposite": point("rectangle: the opposite corner"),
                                "center": point("circle: its centre"),
                                "radius": { "type": "number", "exclusiveMinimum": 0 }
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
            Self::AddExtrusion => object(
                json!({
                    "sketch": reference("The sketch whose regions are extruded"),
                    "distance": { "type": "string", "description": "How far, as an expression; not with through_all" },
                    "through_all": { "type": "boolean", "description": "Cut through the whole body" },
                    "symmetric": { "type": "boolean", "description": "Half the distance each way from the sketch plane" },
                    "operation": { "type": "string", "enum": ["new_body", "add", "remove", "intersect"] },
                    "body": reference("With add, remove or intersect: the body changed"),
                    "reversed": { "type": "boolean", "description": "With an operation: extrude against the sketch plane's normal" },
                    "name": { "type": "string" }
                }),
                &["sketch"],
            ),
            Self::AddHole => object(
                json!({
                    "sketch": reference("The sketch holding the hole centres as free points or circles"),
                    "diameter": { "type": "string", "description": "An expression such as \"6 mm\"" },
                    "depth": { "type": "string", "description": "An expression, or \"through all\"" },
                    "body": reference("The body drilled; by default the one the sketch lies on"),
                    "reversed": { "type": "boolean", "description": "Drill along the sketch plane's normal instead of against it" },
                    "name": { "type": "string" }
                }),
                &["sketch", "diameter", "depth"],
            ),
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
            Self::AddDatumPlane => feature_schemas::datum_plane(),
            Self::AddDatumAxis => feature_schemas::datum_axis(),
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
    AddExtrusion(writing::Extrusion),
    AddHole(writing::Drilling),
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
    AddDatumPlane(features::DatumPlaning),
    AddDatumAxis(features::DatumAxising),
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
            Self::AddExtrusion(extrusion) => writing::extrude(session, work, extrusion),
            Self::AddHole(drilling) => writing::drill(session, work, drilling),
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
            Self::AddDatumPlane(planing) => features::datum_plane(session, work, planing),
            Self::AddDatumAxis(axising) => features::datum_axis(session, work, axising),
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

fn point(description: &str) -> Value {
    json!({
        "type": "array",
        "items": { "type": "number" },
        "minItems": 2,
        "maxItems": 2,
        "description": description
    })
}

fn entity() -> Value {
    json!({
        "type": ["integer", "string"],
        "description": "An entity id, or origin, horizontal_axis or vertical_axis"
    })
}
