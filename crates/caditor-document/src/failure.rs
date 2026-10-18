use caditor_expression::EvalError;
use caditor_kernel::MAX_SIZE;
use caditor_sketch::{DimensionError, SketchError, TextError};

use crate::{
    attachment::AttachCause,
    blend::BlendCause,
    body_pieces::PiecesCause,
    coil::CoilCause,
    combine::CombineCause,
    datum_cause::DatumCause,
    delete_face::DeleteFaceCause,
    document::list_names,
    draft::DraftCause,
    hole::HoleCause,
    import::ImportCause,
    loft::LoftCause,
    mate::MateCause,
    measurement_cause::MeasureCause,
    mirror::MirrorCause,
    move_face::MoveFaceCause,
    movement::MoveCause,
    offset_face::OffsetFaceCause,
    pattern::PatternCause,
    pieces::Unresolved,
    primitive::PrimitiveCause,
    projection::ProjectionCause,
    recompute::{FeatureError, FixTarget},
    reference_image::ImageCause,
    repeat_blend::RepeatCause,
    rib::RibCause,
    scaling::ScaleCause,
    shell::ShellCause,
    solid_cause::SolidCause,
    split::SplitCause,
    split_face::SplitFaceCause,
    sweep_cause::SweepCause,
    thread::ThreadCause,
};

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Cause {
    Panicked,
    Unevaluable(Unevaluable),
    TooBig { what: String },
    Upstream(Upstream),
    Sketch(SketchCause),
    Unmatched(Unmatched),
    Projection(ProjectionCause),
    Attach(AttachCause),
    Import(ImportCause),
    Pieces(PiecesCause),
    Mate(MateCause),
    Scale(ScaleCause),
    Move(MoveCause),
    Image(ImageCause),
    Measure(MeasureCause),
    Datum(DatumCause),
    Rib(RibCause),
    Loft(LoftCause),
    Sweep(SweepCause),
    Solid(SolidCause),
    Coil(CoilCause),
    Primitive(PrimitiveCause),
    SplitFace(SplitFaceCause),
    Split(SplitCause),
    Mirror(MirrorCause),
    Pattern(PatternCause),
    Repeat(RepeatCause),
    Hole(HoleCause),
    Thread(ThreadCause),
    Combine(CombineCause),
    Blend(BlendCause),
    DeleteFace(DeleteFaceCause),
    OffsetFace(OffsetFaceCause),
    MoveFace(MoveFaceCause),
    Draft(DraftCause),
    Shell(ShellCause),
}

impl Cause {
    pub(crate) fn reason(&self) -> String {
        match self {
            Self::Panicked => {
                "caditor ran into an internal error while recomputing this feature.".to_owned()
            }
            Self::Unevaluable(unevaluable) => unevaluable.reason(),
            Self::TooBig { what } => {
                format!("The {what} cannot be more than {} m.", MAX_SIZE / 1_000.0)
            }
            Self::Upstream(upstream) => upstream.reason(),
            Self::Sketch(sketch) => sketch.reason(),
            Self::Unmatched(unmatched) => unmatched.reason(),
            Self::Projection(cause) => cause.reason(),
            Self::Attach(cause) => cause.reason(),
            Self::Import(cause) => cause.reason(),
            Self::Pieces(cause) => cause.reason(),
            Self::Mate(cause) => cause.reason(),
            Self::Scale(cause) => cause.reason(),
            Self::Move(cause) => cause.reason(),
            Self::Image(cause) => cause.reason(),
            Self::Measure(cause) => cause.reason(),
            Self::Datum(cause) => cause.reason(),
            Self::Rib(cause) => cause.reason(),
            Self::Loft(cause) => cause.reason(),
            Self::Sweep(cause) => cause.reason(),
            Self::Solid(cause) => cause.reason(),
            Self::Coil(cause) => cause.reason(),
            Self::Primitive(cause) => cause.reason(),
            Self::SplitFace(cause) => cause.reason(),
            Self::Split(cause) => cause.reason(),
            Self::Mirror(cause) => cause.reason(),
            Self::Pattern(cause) => cause.reason(),
            Self::Repeat(cause) => cause.reason(),
            Self::Hole(cause) => cause.reason(),
            Self::Thread(cause) => cause.reason(),
            Self::Combine(cause) => cause.reason(),
            Self::Blend(blend) => blend.reason(),
            Self::DeleteFace(delete) => delete.reason(),
            Self::OffsetFace(offset) => offset.reason(),
            Self::MoveFace(moving) => moving.reason(),
            Self::Draft(draft) => draft.reason(),
            Self::Shell(shell) => shell.reason(),
        }
    }

    pub(crate) fn remedy(&self) -> String {
        match self {
            Self::Panicked => {
                "Your model is unchanged. Undo the last change, and please report this problem."
                    .to_owned()
            }
            Self::Unevaluable(unevaluable) => unevaluable.remedy(),
            Self::TooBig { what } => {
                format!("Enter a {what} of at most {} m.", MAX_SIZE / 1_000.0)
            }
            Self::Upstream(upstream) => upstream.remedy(),
            Self::Sketch(sketch) => sketch.remedy(),
            Self::Unmatched(unmatched) => unmatched.remedy(),
            Self::Projection(cause) => cause.remedy(),
            Self::Attach(cause) => cause.remedy(),
            Self::Import(cause) => cause.remedy(),
            Self::Pieces(cause) => cause.remedy(),
            Self::Mate(cause) => cause.remedy(),
            Self::Scale(cause) => cause.remedy(),
            Self::Move(cause) => cause.remedy(),
            Self::Image(cause) => cause.remedy(),
            Self::Measure(cause) => cause.remedy(),
            Self::Datum(cause) => cause.remedy(),
            Self::Rib(cause) => cause.remedy(),
            Self::Loft(cause) => cause.remedy(),
            Self::Sweep(cause) => cause.remedy(),
            Self::Solid(cause) => cause.remedy(),
            Self::Coil(cause) => cause.remedy(),
            Self::Primitive(cause) => cause.remedy(),
            Self::SplitFace(cause) => cause.remedy(),
            Self::Split(cause) => cause.remedy(),
            Self::Mirror(cause) => cause.remedy(),
            Self::Pattern(cause) => cause.remedy(),
            Self::Repeat(cause) => cause.remedy(),
            Self::Hole(cause) => cause.remedy(),
            Self::Thread(cause) => cause.remedy(),
            Self::Combine(cause) => cause.remedy(),
            Self::Blend(blend) => blend.remedy(),
            Self::DeleteFace(delete) => delete.remedy(),
            Self::OffsetFace(offset) => offset.remedy(),
            Self::MoveFace(moving) => moving.remedy(),
            Self::Draft(draft) => draft.remedy(),
            Self::Shell(shell) => shell.remedy(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Expected {
    Length(f64),
    Angle(f64),
    Number(f64),
}

impl Expected {
    fn example(self) -> String {
        match self {
            Self::Length(millimetres) => format!("a length, such as {millimetres} mm"),
            Self::Angle(degrees) => format!("an angle, such as {degrees} deg"),
            Self::Number(number) => format!("a plain number, such as {number}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Unevaluable {
    what: String,
    expected: Expected,
    error: EvalError,
}

impl Unevaluable {
    pub(crate) fn reason(&self) -> String {
        format!("The {} cannot be evaluated: {}.", self.what, self.error)
    }

    pub(crate) fn remedy(&self) -> String {
        let what = &self.what;
        match &self.error {
            EvalError::ParameterFailed { name, .. } => {
                format!("Fix {name} under Parameters, or edit the {what}.")
            }
            EvalError::WrongKind { .. } => {
                format!("Edit the {what} so it gives {}.", self.expected.example())
            }
            _ => format!("Edit the {what} or the parameters it uses."),
        }
    }
}

impl FeatureError {
    pub(crate) fn too_big(fix: FixTarget, what: impl Into<String>) -> Self {
        Self::new(Cause::TooBig { what: what.into() }).fixing(fix)
    }

    pub(crate) fn unevaluable(
        feature: crate::document::FeatureId,
        what: impl Into<String>,
        expected: Expected,
        error: EvalError,
    ) -> Self {
        let fix = match &error {
            EvalError::ParameterFailed { id, .. } => FixTarget::Parameter(*id),
            _ => FixTarget::Feature(feature),
        };
        Self::new(Cause::Unevaluable(Unevaluable {
            what: what.into(),
            expected,
            error,
        }))
        .fixing(fix)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Upstream {
    Deleted {
        sketch_attached: bool,
    },
    Suppressed {
        name: String,
    },
    Failed {
        name: String,
    },
    BodyRemoved {
        consumer: String,
        body: String,
    },
    BodyCombined {
        consumer: String,
        body: String,
        kept: String,
    },
    BodyShapeless {
        body: String,
    },
    NoShape {
        name: String,
    },
    PiecesJoined {
        maker: String,
        curve: String,
    },
    PieceGone {
        maker: String,
        curve: String,
        sketch: Option<String>,
    },
}

impl Upstream {
    fn reason(&self) -> String {
        match self {
            Self::Deleted { .. } => "It uses a feature that no longer exists.".to_owned(),
            Self::Suppressed { name } => format!("It uses {name}, which is suppressed."),
            Self::Failed { name } => format!("It uses {name}, which has an error."),
            Self::BodyRemoved { consumer, body } => {
                format!("{consumer} removed the body made by {body}, so it no longer stands.")
            }
            Self::BodyCombined {
                consumer,
                body,
                kept,
            } => format!(
                "{consumer} combined the body made by {body} into the body of {kept}, so it no \
                 longer stands on its own."
            ),
            Self::BodyShapeless { body } => format!("The body made by {body} has no shape."),
            Self::NoShape { name } => {
                format!("It needs the shape of {name}, which is not available.")
            }
            Self::PiecesJoined { maker, curve } => format!(
                "{maker} no longer makes a body of each separate piece, so the piece drawn by \
                 {curve} is not a body of its own."
            ),
            Self::PieceGone { maker, curve, .. } => format!(
                "The piece of {maker} drawn by {curve} is no longer there, so there is no body \
                 to work on."
            ),
        }
    }

    fn remedy(&self) -> String {
        match self {
            Self::Deleted {
                sketch_attached: true,
            } => "Detach the sketch to keep it where it is, or undo the deletion.".to_owned(),
            Self::Deleted {
                sketch_attached: false,
            } => "Undo the deletion, or delete this feature too.".to_owned(),
            Self::Suppressed { name } => {
                format!("Unsuppress {name}, or suppress this feature too.")
            }
            Self::Failed { name } => format!("Fix {name} first."),
            Self::BodyRemoved { consumer, .. } => {
                format!("Move this feature above {consumer}, or suppress or delete {consumer}.")
            }
            Self::BodyCombined { consumer, kept, .. } => {
                format!("Use the body of {kept} instead, or move this feature above {consumer}.")
            }
            Self::BodyShapeless { body } => format!("Fix {body} first."),
            Self::NoShape { name } => format!("Fix {name} first."),
            Self::PiecesJoined { maker, .. } => format!(
                "Set Separate pieces of {maker} back to a body each, or choose another body."
            ),
            Self::PieceGone {
                sketch: Some(sketch),
                ..
            } => format!("Draw the piece again in {sketch}, or choose another body."),
            Self::PieceGone { sketch: None, .. } => "Choose another body.".to_owned(),
        }
    }
}

impl From<Upstream> for Cause {
    fn from(upstream: Upstream) -> Self {
        Self::Upstream(upstream)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum SketchCause {
    NoLength {
        label: String,
    },
    Unevaluable {
        error: SketchError,
    },
    Unsolvable {
        named: Vec<String>,
        unnamed: usize,
        newest: Option<String>,
    },
    Conflict {
        newest: String,
        older: Vec<String>,
    },
    Dimension {
        label: String,
        error: DimensionError,
        expected: Expected,
    },
    Text {
        label: String,
        error: TextError,
    },
    Several(Vec<Cause>),
}

impl SketchCause {
    fn reason(&self) -> String {
        match self {
            Self::NoLength { label } => {
                format!("{label} has no length, so the sketch cannot be solved.")
            }
            Self::Unevaluable { error } => {
                format!("The sketch could not be evaluated: {error}.")
            }
            Self::Unsolvable { named, unnamed, .. } => {
                let (subject, possessive, _) = Self::pronouns(named, *unnamed);
                format!("{subject} could not be solved from {possessive} current shape.")
            }
            Self::Conflict { newest, older } if older.is_empty() => {
                format!("{newest} cannot be satisfied.")
            }
            Self::Conflict { newest, older } => {
                format!("{newest} conflicts with {}.", list_names(older))
            }
            Self::Dimension { label, error, .. } => {
                format!("{label} cannot be evaluated: {error}.")
            }
            Self::Text { label, error } => format!("{label} cannot be drawn: {error}."),
            Self::Several(causes) => format!(
                "The sketch has {} separate problems. {}",
                causes.len(),
                Self::numbered(causes, Cause::reason)
            ),
        }
    }

    fn remedy(&self) -> String {
        match self {
            Self::NoLength { label } => format!(
                "Delete {label}, or move its ends apart and remove any constraint that holds \
                 them together."
            ),
            Self::Unevaluable { .. } => "Undo the last change.".to_owned(),
            Self::Unsolvable {
                named,
                unnamed,
                newest,
            } => {
                let (_, _, them) = Self::pronouns(named, *unnamed);
                match newest {
                    Some(constraint) => format!(
                        "Delete or change {constraint}, the newest constraint on {them}, or undo \
                         the last change."
                    ),
                    None if named.is_empty() && *unnamed == 0 => "Undo the last change.".to_owned(),
                    None => format!("Delete and redraw {them}, or undo the last change."),
                }
            }
            Self::Conflict { older, .. } if older.is_empty() => {
                "Delete or change this constraint, or undo the last change.".to_owned()
            }
            Self::Conflict { .. } => {
                "Delete or change one of these constraints, or undo the last change.".to_owned()
            }
            Self::Dimension {
                error, expected, ..
            } => Self::dimension_remedy(error, *expected),
            Self::Text { label, error } => Self::text_remedy(label, error),
            Self::Several(causes) => format!(
                "Fix each of them. {}",
                Self::numbered(causes, Cause::remedy)
            ),
        }
    }

    fn numbered(causes: &[Cause], pick: fn(&Cause) -> String) -> String {
        causes
            .iter()
            .enumerate()
            .map(|(index, cause)| format!("({}) {}", index + 1, pick(cause)))
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn pronouns(named: &[String], unnamed: usize) -> (String, &'static str, &'static str) {
        let mut names = named.to_vec();
        if unnamed > 0 {
            names.push(format!("{unnamed} more"));
        }
        match named.len() + unnamed {
            0 => ("The sketch".to_owned(), "its", "it"),
            1 => (list_names(&names), "its", "it"),
            _ => (list_names(&names), "their", "them"),
        }
    }

    fn text_remedy(label: &str, error: &TextError) -> String {
        match error {
            TextError::Height(EvalError::ParameterFailed { name, .. })
            | TextError::Angle(EvalError::ParameterFailed { name, .. }) => {
                format!("Fix {name} under Parameters, or select {label} and edit it with Text.")
            }
            TextError::Height(_) | TextError::HeightNotPositive | TextError::HeightTooLarge => {
                format!("Select {label} and give it a height such as 5 mm with Text.")
            }
            TextError::Angle(_) | TextError::AngleNotFinite => {
                format!("Select {label} and give it an angle such as 0 deg with Text.")
            }
            TextError::Blank | TextError::TooLong { .. } | TextError::ControlCharacter => {
                format!("Select {label} and change its letters with Text.")
            }
        }
    }

    fn dimension_remedy(error: &DimensionError, expected: Expected) -> String {
        match error {
            DimensionError::Evaluation(EvalError::ParameterFailed { name, .. }) => {
                format!("Fix {name} under Parameters, or edit this dimension.")
            }
            DimensionError::Evaluation(EvalError::WrongKind { .. }) => {
                format!("Edit the dimension so it gives {}.", expected.example())
            }
            DimensionError::Negative => "Edit the dimension so it gives zero or more.".to_owned(),
            DimensionError::NotPositive
            | DimensionError::DiameterNotPositive
            | DimensionError::ArcLengthNotPositive => {
                "Edit the dimension so it gives more than zero.".to_owned()
            }
            DimensionError::SweepOutsideTurn => {
                "Edit the dimension so it gives an angle between 0 and 360 deg.".to_owned()
            }
            DimensionError::RhoOutOfRange => {
                "Edit the dimension so it gives a rho between 0.01 and 0.99.".to_owned()
            }
            DimensionError::TooLong => {
                "Edit the dimension so it gives a length the model can hold.".to_owned()
            }
            DimensionError::Evaluation(_) | DimensionError::NotFinite => {
                "Edit the dimension or the parameters it uses.".to_owned()
            }
        }
    }
}

impl From<SketchCause> for Cause {
    fn from(sketch: SketchCause) -> Self {
        Self::Sketch(sketch)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Chosen {
    Edges,
    Faces,
    FacesToOpen,
    FacesToMove,
    FacesToDraft,
    FacesWithOwnAngle,
    FacesToDelete,
    FacesToMirror,
    FacesToSplit,
}

impl Chosen {
    fn singular(self) -> &'static str {
        match self {
            Self::Edges => "chosen edge",
            Self::Faces => "chosen face",
            Self::FacesToOpen => "face to open",
            Self::FacesToMove => "face to move",
            Self::FacesToDraft => "face to draft",
            Self::FacesWithOwnAngle => "face given its own draft angle",
            Self::FacesToDelete => "face to delete",
            Self::FacesToMirror => "face to mirror",
            Self::FacesToSplit => "face to split",
        }
    }

    fn plural(self) -> &'static str {
        match self {
            Self::Edges => "chosen edges",
            Self::Faces => "chosen faces",
            Self::FacesToOpen => "faces to open",
            Self::FacesToMove => "faces to move",
            Self::FacesToDraft => "faces to draft",
            Self::FacesWithOwnAngle => "faces given their own draft angle",
            Self::FacesToDelete => "faces to delete",
            Self::FacesToMirror => "faces to mirror",
            Self::FacesToSplit => "faces to split",
        }
    }

    fn things(self) -> &'static str {
        match self {
            Self::Edges => "edges",
            _ => "faces",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Unmatched {
    pub chosen: Chosen,
    pub unresolved: Unresolved,
    pub body: String,
}

impl Unmatched {
    fn reason(&self) -> String {
        let Self {
            chosen,
            unresolved,
            body,
        } = self;
        let things = chosen.things();
        match unresolved {
            Unresolved::Missing(1) => format!(
                "A {} is no longer part of the body of {body}.",
                chosen.singular()
            ),
            Unresolved::Missing(missing) => format!(
                "{missing} {} are no longer part of the body of {body}.",
                chosen.plural()
            ),
            Unresolved::Unrelated(1) => format!(
                "A {} now matches several separate {things} of the body of {body}.",
                chosen.singular()
            ),
            Unresolved::Unrelated(unrelated) => format!(
                "{unrelated} {} now match several separate {things} of the body of {body}.",
                chosen.plural()
            ),
        }
    }

    fn remedy(&self) -> String {
        format!(
            "Choose the {} again, or undo the change that removed them.",
            self.chosen.things()
        )
    }
}

impl From<Unmatched> for Cause {
    fn from(unmatched: Unmatched) -> Self {
        Self::Unmatched(unmatched)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Near {
    pub faces: Vec<String>,
    pub edge: Option<String>,
}

impl std::fmt::Display for Near {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (self.faces.is_empty(), &self.edge) {
            (false, _) => write!(formatter, " near {}", list_names(&self.faces)),
            (true, Some(edge)) => write!(formatter, " near {edge}"),
            (true, None) => Ok(()),
        }
    }
}
