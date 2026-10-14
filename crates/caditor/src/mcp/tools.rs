use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};

use crate::{
    mcp::{
        files::{self, FileError},
        reading::{self, RenderError},
        references::ReferenceError,
        session::{Cancelled, ChangeError, Session, Work},
        sketching::{self, SketchToolError},
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
    AddExtrusion,
    AddHole,
    Undo,
    Redo,
    Save,
    Export,
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
    AddExtrusion,
    AddHole,
    Undo,
    Redo,
    Save,
    Export
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
    SketchTools(#[from] SketchToolError),
    #[error(transparent)]
    Modelling(#[from] ModellingError),
    #[error(transparent)]
    Files(#[from] FileError),
    #[error(transparent)]
    Render(#[from] RenderError),
    #[error(transparent)]
    Cancelled(#[from] Cancelled),
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
            Self::AddExtrusion => "add_extrusion",
            Self::AddHole => "add_hole",
            Self::Undo => "undo",
            Self::Redo => "redo",
            Self::Save => "save",
            Self::Export => "export",
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
            Self::AddExtrusion => "Extrude",
            Self::AddHole => "Hole",
            Self::Undo => "Undo",
            Self::Redo => "Redo",
            Self::Save => "Save",
            Self::Export => "Export",
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
                "Adds an empty sketch at the end of the tree on a principal plane (XY, XZ or YZ), \
                 on a flat face of a body (a key from list_faces), on a datum plane (datum), or \
                 on a plane of a coordinate system (frame with plane). One undo step. Returns \
                 the sketch's plane: sketch x and y map to its x_axis and y_axis from its origin."
            }
            Self::AddSketchGeometry => {
                "Draws points, lines, polylines, rectangles, circles, arcs, slots, arc slots, \
                 polygons, splines and ellipses into a sketch, in sketch coordinates (mm). A \
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
            Self::EditSketchConstraint => {
                "Changes a dimension's value (an expression with units and parameters, such as \
                 \"30 mm\" or \"width / 2\") and/or turns a constraint on or off; a dimension \
                 turned off is a reference, measured but no longer driving. One undo step."
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
            | Self::RenderView => Effect::Reads,
            Self::AddSketch
            | Self::AddSketchGeometry
            | Self::AddSketchConstraints
            | Self::AddExtrusion
            | Self::AddHole => Effect::Adds,
            Self::SetParameter
            | Self::RemoveSketchItems
            | Self::EditSketchConstraint
            | Self::Undo
            | Self::Redo => Effect::Changes,
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
                                "kind": { "type": "string", "enum": ["point", "line", "polyline", "rectangle", "circle", "arc", "slot", "arc_slot", "polygon", "spline", "ellipse"] },
                                "at": point("point: where it is"),
                                "from": point("line: its start; slot: the first end's centre; arc_slot: where the centre line starts"),
                                "to": point("line: its end; slot: the second end's centre; arc_slot: where the centre line ends, counter-clockwise from from"),
                                "points": { "type": "array", "items": point("A corner"), "description": "polyline: its corners in order; spline: its points in order" },
                                "closed": { "type": "boolean", "description": "polyline: join the last corner to the first; spline: close it into a loop (three points at least)" },
                                "corner": point("rectangle: one corner; polygon: a vertex, which fixes its size and turn"),
                                "opposite": point("rectangle: the opposite corner"),
                                "center": point("circle, arc, arc_slot, polygon, ellipse: the centre"),
                                "radius": { "type": "number", "exclusiveMinimum": 0 },
                                "start": point("arc: where it starts"),
                                "end": point("arc: where it ends; with center only its direction from the centre counts"),
                                "through": point("arc: a point it passes through, instead of center"),
                                "clockwise": { "type": "boolean", "description": "arc with center: run clockwise from start to end instead of counter-clockwise" },
                                "width": { "type": "number", "exclusiveMinimum": 0, "description": "slot, arc_slot: the full width across" },
                                "sides": { "type": "integer", "minimum": 3, "maximum": 64, "description": "polygon: the number of sides, 6 by default" },
                                "side_middle": point("polygon: the middle of its first side, instead of corner"),
                                "form": { "type": "string", "enum": ["fit", "control"], "description": "spline: through its points (fit, the default) or shaped by them as control points" },
                                "major_end": point("ellipse: the end of its major axis"),
                                "minor_radius": { "type": "number", "exclusiveMinimum": 0, "description": "ellipse: half its width across the minor axis" }
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
            Self::EditSketchConstraint => object(
                json!({
                    "sketch": reference("The sketch, by feature id or name"),
                    "constraint": { "type": "integer", "description": "The constraint or dimension id from get_sketch" },
                    "value": { "type": "string", "description": "A dimension's new value as an expression such as \"80 mm\", \"30 deg\" or \"length / 2\"" },
                    "active": { "type": "boolean", "description": "false turns the constraint off (a dimension becomes a reference), true turns it on" }
                }),
                &["sketch", "constraint"],
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
            Self::AddExtrusion => Call::AddExtrusion(self.arguments(arguments)?),
            Self::AddHole => Call::AddHole(self.arguments(arguments)?),
            Self::Undo => Call::Undo(self.arguments(arguments)?),
            Self::Redo => Call::Redo(self.arguments(arguments)?),
            Self::Save => Call::Save(self.arguments(arguments)?),
            Self::Export => Call::Export(self.arguments(arguments)?),
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
    AddExtrusion(writing::Extrusion),
    AddHole(writing::Drilling),
    Undo(reading::Nothing),
    Redo(reading::Nothing),
    Save(reading::Nothing),
    Export(files::Exporting),
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
            Self::AddExtrusion(extrusion) => writing::extrude(session, work, extrusion),
            Self::AddHole(drilling) => writing::drill(session, work, drilling),
            Self::Undo(_) => writing::undo(session, work),
            Self::Redo(_) => writing::redo(session, work),
            Self::Save(_) => files::save(session).map(Answer::of),
            Self::Export(exporting) => files::export(session, work, &exporting).map(Answer::of),
        }
    }
}

fn object(properties: Value, required: &[&str]) -> Value {
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false
    })
}

fn reference(description: &str) -> Value {
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
