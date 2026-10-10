use std::collections::BTreeSet;

use caditor_expression::format_number;
use caditor_geometry::{Plane, Ray, Vector3};
use caditor_kernel::{
    Cylinder, FaceId, FaceName, FaceReference, FaceSplitError, Profile, ProfileError, Selection,
    Solid, Surface, WrapError, split_faces, wrap_regions,
};

use crate::{
    attachment::FaceAttachment,
    datum::{AxisReference, Resolver, capitalized, describe_axis, feature_name},
    describe::{describe_origin, describe_surface},
    document::{Feature, FeatureId},
    origins,
    pieces::{Resolution, Unresolved, pieces_of_one_face, tally},
    recompute::{CancelToken, Failure, FeatureResult, Inputs, SketchResult},
    solid::{SolidResult, profile_curves},
    split::{
        HalfSpaceError, MixedCurves, SplitAlong, Sweep, SweptError, half_space_solid,
        is_open_chain, mixed_curves,
    },
    surface_tool::{SurfaceToolError, surface_tool},
    tolerance, trouble,
};

#[derive(Debug, Clone, PartialEq, Default)]
pub enum SplitCarry {
    #[default]
    Square,
    Along(Box<AxisReference>),
    Wrapped,
}

impl SplitCarry {
    pub fn direction(&self) -> Option<&AxisReference> {
        match self {
            Self::Along(axis) => Some(axis),
            Self::Square | Self::Wrapped => None,
        }
    }

    pub fn direction_mut(&mut self) -> Option<&mut AxisReference> {
        match self {
            Self::Along(axis) => Some(axis),
            Self::Square | Self::Wrapped => None,
        }
    }

    pub fn heap_size(&self) -> usize {
        self.direction()
            .map_or(0, |axis| size_of::<AxisReference>() + axis.heap_size())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SplitFace {
    pub body: FeatureId,
    pub faces: Vec<FaceReference>,
    pub along: SplitAlong,
    pub carry: SplitCarry,
}

impl SplitFace {
    pub fn heap_size(&self) -> usize {
        size_of_val(self.faces.as_slice())
            + self
                .faces
                .iter()
                .map(FaceReference::heap_size)
                .sum::<usize>()
            + self.along.heap_size()
            + self.carry.heap_size()
    }

    pub fn with_along(&self, along: SplitAlong) -> Self {
        let carry = match along.sketch() {
            Some(_) => self.carry.clone(),
            None => SplitCarry::Square,
        };
        Self {
            along,
            carry,
            ..self.clone()
        }
    }

    pub fn direction(&self) -> Option<&AxisReference> {
        self.carry.direction()
    }

    pub fn is_wrapped(&self) -> bool {
        matches!(self.carry, SplitCarry::Wrapped)
    }

    pub fn direction_body(&self) -> Option<FeatureId> {
        self.direction().and_then(AxisReference::body)
    }

    pub fn direction_datum(&self) -> Option<FeatureId> {
        self.direction().and_then(AxisReference::datum)
    }

    pub fn direction_frame(&self) -> Option<FeatureId> {
        self.direction().and_then(AxisReference::frame)
    }

    pub fn direction_sketch(&self) -> Option<FeatureId> {
        self.direction().and_then(AxisReference::sketch)
    }

    pub fn features(&self) -> BTreeSet<FeatureId> {
        let mut used = BTreeSet::from([self.body]);
        used.extend(self.along.datum());
        used.extend(self.along.body());
        used.extend(self.along.sketch());
        used.extend(self.direction_datum());
        used.extend(self.direction_body());
        used.extend(self.direction_sketch());
        used
    }

    pub fn origin_features(&self) -> BTreeSet<FeatureId> {
        let mut origins: BTreeSet<FeatureId> =
            self.faces.iter().flat_map(origins::of_face).collect();
        origins.extend(self.along.origin_features());
        origins.extend(
            self.direction()
                .into_iter()
                .flat_map(AxisReference::origin_features),
        );
        origins
    }

    pub fn resolutions(&self, solid: &Solid) -> Vec<Resolution<FaceId>> {
        self.faces
            .iter()
            .map(|reference| {
                Resolution::of(reference.resolve(solid), |pieces| {
                    pieces_of_one_face(solid, pieces)
                })
            })
            .collect()
    }

    pub fn resolve(&self, solid: &Solid) -> Result<Vec<FaceId>, Unresolved> {
        tally(self.resolutions(solid))
    }
}

struct Context<'a> {
    resolver: Resolver<'a>,
    body_name: String,
    tool: String,
}

impl Context<'_> {
    fn error(&self, reason: String, remedy: &str) -> Failure {
        self.resolver.own_error(reason, remedy)
    }

    fn misses(&self) -> Failure {
        self.error(
            format!("{} does not cross any of the chosen faces.", self.tool),
            "Choose faces it crosses, or move it so it runs across them.",
        )
    }

    fn unbuildable(&self, error: &dyn std::fmt::Display) -> Failure {
        log::warn!("{} could not be built: {error}", self.resolver.feature.name);
        self.error(
            format!(
                "The faces of {} could not be divided along {}.",
                self.body_name, self.tool
            ),
            "Move it slightly, so it does not run exactly along an edge.",
        )
    }

    fn unresolved(&self, unresolved: Unresolved) -> Failure {
        let body = &self.body_name;
        let reason = match unresolved {
            Unresolved::Missing(1) => {
                format!("A face to split is no longer part of the body of {body}.")
            }
            Unresolved::Missing(missing) => {
                format!("{missing} faces to split are no longer part of the body of {body}.")
            }
            Unresolved::Unrelated(1) => {
                format!("A face to split now matches several separate faces of the body of {body}.")
            }
            Unresolved::Unrelated(unrelated) => format!(
                "{unrelated} faces to split now match several separate faces of the body of \
                 {body}."
            ),
        };
        self.error(
            reason,
            "Choose the faces again, or undo the change that removed them.",
        )
    }

    fn split_failure(&self, solid: &Solid, tool: &Solid, error: FaceSplitError) -> Failure {
        match error {
            FaceSplitError::Cancelled(_) => Failure::Cancelled,
            FaceSplitError::NoFaces => self.error(
                "No face is chosen to split.".to_owned(),
                "Choose the faces to split.",
            ),
            FaceSplitError::Undivided => self.misses(),
            FaceSplitError::Boolean(error) => {
                let document = self.resolver.inputs.document;
                let trouble = trouble::boolean_trouble(document, [solid, tool], &error, "Move it");
                log::warn!("{} could not be built: {error}", self.resolver.feature.name);
                self.error(
                    trouble.reason(format!(
                        "The faces of {} could not be divided along {}.",
                        self.body_name, self.tool
                    )),
                    &trouble.remedy,
                )
                .placed(trouble.place)
            }
        }
    }

    fn surface_failure(&self, face: &FaceAttachment, error: SurfaceToolError) -> Failure {
        if matches!(error, SurfaceToolError::Misses) {
            return self.misses();
        }
        match error.words(self.resolver.inputs.document, face) {
            Some(words) => self.error(words.reason, &words.remedy),
            None => self.unbuildable(&error),
        }
    }

    fn swept_failure(&self, sketch: FeatureId, error: SweptError) -> Failure {
        let name = feature_name(self.resolver.inputs.document, sketch);
        let (reason, remedy) = match error {
            SweptError::NoCurves => (
                format!("{name} has no curve to split along."),
                format!("Draw a curve across the faces in {name}."),
            ),
            SweptError::NotOneChain | SweptError::Profile(_) => (
                format!(
                    "The curves of {name} form neither one open chain nor closed outlines, so \
                     they do not divide the faces into sides."
                ),
                format!(
                    "Leave one open chain of curves in {name}, or close its outlines, making \
                     other curves construction geometry."
                ),
            ),
            SweptError::CrossesItself => (
                format!(
                    "The curve of {name}, carried on straight past its ends, crosses itself, so \
                     it does not divide the faces into two sides."
                ),
                format!(
                    "Lengthen the curve of {name} so it runs past the faces, or draw it without \
                     crossing itself."
                ),
            ),
            other => return self.unbuildable(&other),
        };
        self.resolver.error(reason, remedy, sketch)
    }
}

fn tool_words(inputs: &Inputs<'_>, along: &SplitAlong) -> String {
    match along {
        SplitAlong::Plane(_) => "The plane".to_owned(),
        SplitAlong::Body(tool) => feature_name(inputs.document, *tool),
        SplitAlong::Sketch(sketch) => {
            format!("The curve of {}", feature_name(inputs.document, *sketch))
        }
        SplitAlong::Surface(face) => capitalized(&describe_surface(inputs.document, face)),
    }
}

enum Tool<'a> {
    Made(Solid),
    Body(&'a Solid),
    Twice { outlines: Solid, chain: Solid },
}

fn sketch_result<'a>(
    context: &Context<'_>,
    inputs: &'a Inputs<'_>,
    sketch: FeatureId,
) -> Result<&'a SketchResult, Failure> {
    inputs
        .features
        .get(&sketch)
        .and_then(|result| result.sketch())
        .ok_or_else(|| {
            let name = feature_name(inputs.document, sketch);
            context.resolver.error(
                format!("It uses {name}, which has an error."),
                format!("Fix {name} first."),
                sketch,
            )
        })
}

fn carried_direction(
    context: &Context<'_>,
    definition: &SplitFace,
    plane: &Plane,
    sketch: FeatureId,
) -> Result<Option<Vector3>, Failure> {
    let Some(reference) = definition.direction() else {
        return Ok(None);
    };
    let axis = context.resolver.axis(reference)?;
    let normal = plane.normal();
    let direction = axis.direction().normalize();
    if tolerance::perpendicular(direction, normal) {
        let document = context.resolver.inputs.document;
        return Err(context.error(
            format!(
                "{} runs along the plane of {}, so its curves cannot be carried along it.",
                capitalized(&describe_axis(document, reference)),
                feature_name(document, sketch)
            ),
            "Choose an edge, axis or line that leaves the sketch plane, or carry the curves \
             square to the sketch.",
        ));
    }
    Ok(Some(if direction.dot(normal) < 0.0 {
        -direction
    } else {
        direction
    }))
}

fn sketch_tool<'a>(
    context: &Context<'_>,
    definition: &SplitFace,
    solid: &Solid,
    sketch: FeatureId,
) -> Result<Tool<'a>, Failure> {
    let result = sketch_result(context, context.resolver.inputs, sketch)?;
    let geometry = &result.geometry;
    let plane = geometry.plane();
    let sweep = Sweep {
        plane,
        direction: carried_direction(context, definition, &plane, sketch)?,
        feature: context.resolver.feature.id().raw(),
    };
    let failed = |error| context.swept_failure(sketch, error);
    if is_open_chain(geometry) {
        return sweep
            .half_space(solid, profile_curves(geometry), false)
            .map(Tool::Made)
            .map_err(failed);
    }
    match mixed_curves(geometry) {
        Some(MixedCurves { chain, outlines }) => Ok(Tool::Twice {
            outlines: sweep.outlines(solid, &outlines).map_err(failed)?,
            chain: sweep.half_space(solid, chain, false).map_err(failed)?,
        }),
        None => sweep
            .outlines(solid, &profile_curves(geometry))
            .map(Tool::Made)
            .map_err(failed),
    }
}

fn wrapping_cylinder(
    context: &Context<'_>,
    solid: &Solid,
    faces: &[FaceId],
    sketch: &str,
) -> Result<Cylinder, Failure> {
    let document = context.resolver.inputs.document;
    let mut cylinder: Option<Cylinder> = None;
    for face in faces.iter().filter_map(|face| solid.face(*face)) {
        let Surface::Cylinder(found) = face.surface() else {
            return Err(context.error(
                format!(
                    "{} is not cylindrical, so the curves of {sketch} cannot be wrapped onto it.",
                    capitalized(&describe_origin(document, face.origin()))
                ),
                "Choose faces of one cylinder only, or carry the curves square to the sketch.",
            ));
        };
        match cylinder {
            None => cylinder = Some(*found),
            Some(first) if same_cylinder(&first, found) => {}
            Some(_) => {
                return Err(context.error(
                    format!(
                        "The chosen faces of {} do not lie on one cylinder, so the curves of \
                         {sketch} cannot be wrapped onto them all.",
                        context.body_name
                    ),
                    "Choose faces of one cylinder only, or split the others in another \
                     feature.",
                ));
            }
        }
    }
    cylinder.ok_or_else(|| {
        context.error(
            "No face is chosen to split.".to_owned(),
            "Choose the faces to split.",
        )
    })
}

fn same_cylinder(first: &Cylinder, second: &Cylinder) -> bool {
    let axis = |cylinder: &Cylinder| Ray::new(cylinder.frame().origin(), cylinder.frame().normal());
    match (axis(first), axis(second)) {
        (Some(first_axis), Some(second_axis)) => {
            tolerance::same_line(first_axis, second_axis)
                && (first.radius() - second.radius()).abs() <= tolerance::POSITION_TOLERANCE
        }
        _ => false,
    }
}

fn wrapped_tool<'a>(
    context: &Context<'_>,
    solid: &Solid,
    faces: &[FaceId],
    sketch: FeatureId,
) -> Result<Tool<'a>, Failure> {
    let inputs = context.resolver.inputs;
    let result = sketch_result(context, inputs, sketch)?;
    let geometry = &result.geometry;
    let plane = geometry.plane();
    let name = feature_name(inputs.document, sketch);
    let cylinder = wrapping_cylinder(context, solid, faces, &name)?;
    if !tolerance::perpendicular(plane.normal(), cylinder.frame().normal()) {
        return Err(context.error(
            format!(
                "{name} does not lie on a plane along the axis of the chosen faces, so its \
                 curves cannot be wrapped round them."
            ),
            "Place the sketch on a plane parallel to the cylinder's axis, such as one touching \
             it, or carry the curves square to the sketch.",
        ));
    }
    let curves = profile_curves(geometry);
    if curves.is_empty() {
        return Err(context.swept_failure(sketch, SweptError::NoCurves));
    }
    let open = || {
        context.resolver.error(
            format!(
                "Only closed outlines can be wrapped onto a cylinder, and the curves of {name} \
                 are not all closed."
            ),
            format!(
                "Close the outlines in {name}, making other curves construction geometry, or \
                 carry its curves square to the sketch."
            ),
            sketch,
        )
    };
    if is_open_chain(geometry) || mixed_curves(geometry).is_some() {
        return Err(open());
    }
    let regions = Profile::new(&curves)
        .and_then(|profile| profile.select(&Selection::EvenDepth))
        .map_err(|error| match error {
            ProfileError::NoClosedProfile { .. } => open(),
            other => context.swept_failure(sketch, SweptError::Profile(other)),
        })?;
    let feature = context.resolver.feature.id().raw();
    wrap_regions(&plane, &regions, &cylinder, feature)
        .map(Tool::Made)
        .map_err(|error| match error {
            WrapError::Cancelled(_) => Failure::Cancelled,
            WrapError::BeyondFullTurn {
                span,
                circumference,
            } => context.resolver.error(
                format!(
                    "The outlines of {name} reach {} mm round the cylinder, which is only {} mm \
                     round, so wrapped they would overlap.",
                    format_number(span),
                    format_number(circumference)
                ),
                format!(
                    "Keep the outlines of {name} within {} mm across the cylinder's axis.",
                    format_number(circumference)
                ),
                sketch,
            ),
            other => context.unbuildable(&other),
        })
}

fn tool<'a>(
    context: &'a Context<'_>,
    definition: &SplitFace,
    solid: &Solid,
    faces: &[FaceId],
) -> Result<Tool<'a>, Failure> {
    let feature = context.resolver.feature.id().raw();
    match &definition.along {
        SplitAlong::Plane(reference) => {
            let plane = context.resolver.plane(reference)?;
            half_space_solid(solid, &plane, false, feature)
                .map(Tool::Made)
                .map_err(|error| match error {
                    HalfSpaceError::Misses => context.misses(),
                    other => context.unbuildable(&other),
                })
        }
        SplitAlong::Body(tool) if *tool == definition.body => Err(context.error(
            format!(
                "The faces of {} cannot be split along its own body.",
                context.body_name
            ),
            "Choose another body to split them along.",
        )),
        SplitAlong::Body(tool) => context.resolver.body(*tool).map(Tool::Body),
        SplitAlong::Sketch(sketch) if definition.is_wrapped() => {
            wrapped_tool(context, solid, faces, *sketch)
        }
        SplitAlong::Sketch(sketch) => sketch_tool(context, definition, solid, *sketch),
        SplitAlong::Surface(face) => {
            let holder = context.resolver.body(face.body)?;
            surface_tool(holder, &face.face, solid, false, feature)
                .map(|made| Tool::Made(made.solid))
                .map_err(|error| context.surface_failure(face, error))
        }
    }
}

fn divided(
    context: &Context<'_>,
    solid: &Solid,
    faces: &[FaceId],
    tool: &Solid,
) -> Result<Option<Solid>, Failure> {
    match split_faces(solid, faces, tool, context.resolver.feature.id().raw()) {
        Ok(split) => Ok(Some(split)),
        Err(FaceSplitError::Undivided) => Ok(None),
        Err(error) => Err(context.split_failure(solid, tool, error)),
    }
}

fn split_twice(
    context: &Context<'_>,
    solid: &Solid,
    faces: &[FaceId],
    tools: [&Solid; 2],
    cancel: &CancelToken,
) -> Result<Solid, Failure> {
    let [outlines, chain] = tools;
    let unchosen: BTreeSet<FaceName> = solid
        .faces()
        .filter(|(id, _)| !faces.contains(id))
        .map(|(_, face)| face.name())
        .collect();
    let first = divided(context, solid, faces, outlines)?;
    if cancel.is_cancelled() {
        return Err(Failure::Cancelled);
    }
    let (base, chosen) = match &first {
        Some(split) => (
            split,
            split
                .faces()
                .filter(|(_, face)| !unchosen.contains(&face.name()))
                .map(|(id, _)| id)
                .collect(),
        ),
        None => (solid, faces.to_vec()),
    };
    match divided(context, base, &chosen, chain)? {
        Some(split) => Ok(split),
        None => first.ok_or_else(|| context.misses()),
    }
}

pub(crate) fn evaluate(
    feature: &Feature,
    definition: &SplitFace,
    inputs: &Inputs<'_>,
    cancel: &CancelToken,
) -> Result<FeatureResult, Failure> {
    let body_name = inputs
        .document
        .feature(definition.body)
        .map(|body| body.name.clone())
        .unwrap_or_default();
    let context = Context {
        resolver: Resolver { feature, inputs },
        body_name,
        tool: tool_words(inputs, &definition.along),
    };
    let Some(solid) = inputs.body(definition.body) else {
        return Err(inputs.missing_body(definition.body));
    };
    let faces = definition
        .resolve(solid)
        .map_err(|unresolved| context.unresolved(unresolved))?;
    if cancel.is_cancelled() {
        return Err(Failure::Cancelled);
    }
    let tool = tool(&context, definition, solid, &faces)?;
    if cancel.is_cancelled() {
        return Err(Failure::Cancelled);
    }
    let split = match &tool {
        Tool::Made(made) => divided(&context, solid, &faces, made)?,
        Tool::Body(body) => divided(&context, solid, &faces, body)?,
        Tool::Twice { outlines, chain } => Some(split_twice(
            &context,
            solid,
            &faces,
            [outlines, chain],
            cancel,
        )?),
    }
    .ok_or_else(|| context.misses())?;
    Ok(FeatureResult::Solid(SolidResult::new(
        definition.body,
        split,
    )))
}
