use std::borrow::Cow;

use caditor_document::{
    BodyOperation, Document, Edit, Evaluation, FeatureId, FeatureKind, Pipe, SweepPath,
    Transaction, capitalized, pipe_size_name,
};
use caditor_expression::Expression;
use caditor_kernel::{EdgeName, PipeShape};
use caditor_sketch::EntityId;

use crate::{
    commands::{Command, Reason},
    editing::{self, EditingCommand},
    field::{self, CheckError},
    last_values::{Remembered, Starts},
    model::{Action, Model},
    pipe_panel,
    queued::{self, Pending},
    reference_picking::{Picking, Slot},
    selection::{Pickable, Selection},
    sweep_tools::{self, PathPick, SweepError},
    units::LengthUnit,
};

pub const TITLE: &str = "Pipe";
pub const DEFAULT_SIZE: f64 = 10.0;
pub const DEFAULT_BEND: f64 = 10.0;
pub const DEFAULT_WALL: f64 = 1.0;

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum PipeError {
    #[error(transparent)]
    Path(#[from] SweepError),
    #[error("The feature no longer exists")]
    Gone { feature: FeatureId },
    #[error(transparent)]
    Check(#[from] CheckError),
}

impl Reason for PipeError {
    fn reason(&self) -> Cow<'static, str> {
        Cow::Owned(self.to_string())
    }

    fn pending(&self) -> Option<Pending> {
        match self {
            Self::Path(error) => error.pending(),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PipeStart {
    pub path: PathPick,
}

pub fn source(
    model: &Model,
    selection: &Selection,
    rows: &[FeatureId],
) -> Result<PipeStart, PipeError> {
    let document = model.document();
    let mut curves: Vec<(FeatureId, EntityId)> = Vec::new();
    let mut edges: Vec<(FeatureId, EdgeName)> = Vec::new();

    for pickable in selection.in_pick_order() {
        match pickable {
            Pickable::SketchEntity { feature, entity }
                if sweep_tools::is_path_curve(document, feature, entity) =>
            {
                curves.push((feature, entity));
            }
            Pickable::Edge { body, edge } => edges.push((body, edge)),
            _ => {}
        }
    }

    let helix =
        sweep_tools::helix_path(document, selection, !curves.is_empty() || !edges.is_empty())?;
    let path = match (curves.first(), edges.first()) {
        (None, None) => match helix {
            Some(helix) => PathPick::Helix { helix },
            None => path_from_rows(document, rows)?,
        },
        (Some(_), Some(_)) => return Err(SweepError::CurvesAndEdges.into()),
        (Some((sketch, _)), None) => {
            if curves.iter().any(|(other, _)| other != sketch) {
                return Err(SweepError::SeveralSketches.into());
            }
            PathPick::Curves {
                sketch: *sketch,
                curves: curves.iter().map(|(_, curve)| *curve).collect(),
            }
        }
        (None, Some((body, _))) => {
            if edges.iter().any(|(other, _)| other != body) {
                return Err(SweepError::PathEdgesOfSeveralBodies.into());
            }
            PathPick::Edges {
                body: *body,
                edges: edges.iter().map(|(_, edge)| *edge).collect(),
            }
        }
    };
    Ok(PipeStart { path })
}

fn path_from_rows(document: &Document, rows: &[FeatureId]) -> Result<PathPick, PipeError> {
    let bar = document.bar_index();
    let mut helices: Vec<FeatureId> = Vec::new();
    let mut sketches: Vec<FeatureId> = Vec::new();

    for row in rows {
        if !document
            .feature_index(*row)
            .is_some_and(|position| position < bar)
        {
            continue;
        }
        if crate::helix_tools::is_helix(document, *row) {
            if !helices.contains(row) {
                helices.push(*row);
            }
        } else if document
            .feature(*row)
            .is_some_and(|feature| feature.kind.sketch().is_some())
            && !sweep_tools::path_curves_of(document, *row).is_empty()
            && !sketches.contains(row)
        {
            sketches.push(*row);
        }
    }
    match (helices.as_slice(), sketches.as_slice()) {
        ([], []) => Ok(PathPick::Unchosen),
        ([helix], []) => Ok(PathPick::Helix { helix: *helix }),
        ([_, ..], _) => Err(SweepError::HelixAndOthers.into()),
        ([], [sketch]) => Ok(PathPick::Curves {
            sketch: *sketch,
            curves: sweep_tools::path_curves_of(document, *sketch),
        }),
        ([], [_, _, ..]) => Err(SweepError::SeveralSketches.into()),
    }
}

pub fn create(
    document: &Document,
    evaluation: &Evaluation,
    start: &PipeStart,
    starts: &Starts,
) -> Result<(Transaction, FeatureId), PipeError> {
    let path = sweep_tools::captured(evaluation, &start.path)?;
    let operation = path
        .body()
        .map_or(BodyOperation::NewBody, BodyOperation::Add);
    let name = editing::next_feature_name(document, TITLE);

    let pipe = Pipe {
        operation,
        ..Pipe::new(
            path,
            starts.length(Remembered::PipeSize, DEFAULT_SIZE),
            starts.length(Remembered::PipeBend, DEFAULT_BEND),
        )
    };
    let mut transaction = document.transaction(format!("Create {name}"));

    let feature = transaction.add_feature(name, FeatureKind::Pipe(pipe));
    Ok((transaction.finish(), feature))
}

pub fn create_actions(model: &Model, start: &PipeStart) -> Vec<Action> {
    match create(
        model.document(),
        model.evaluation(),
        start,
        &Starts::of(model),
    ) {
        Ok((transaction, feature)) => {
            let mut actions = vec![
                Action::Apply(transaction),
                Action::Editing(EditingCommand::OpenSolid(feature)),
            ];

            if start.path == PathPick::Unchosen {
                actions.push(Action::Editing(EditingCommand::Pick(Picking::new(
                    feature,
                    Slot::SweepPath,
                ))));
            }
            actions
        }
        Err(reason) => vec![queued::refused(Command::Pipe, TITLE, &reason)],
    }
}

pub fn edit(document: &Document, feature: FeatureId, pipe: Pipe) -> Option<Transaction> {
    let name = &document.feature(feature)?.name;
    let size_caption = capitalized(pipe_size_name(pipe.shape));
    let mut fields: Vec<(&str, &Expression)> = vec![
        (size_caption.as_str(), &pipe.size),
        (pipe_panel::BEND_RADIUS, &pipe.bend),
    ];

    if let Some(wall) = &pipe.wall {
        fields.push((pipe_panel::WALL_THICKNESS, wall));
    }

    let transaction = Transaction::single(
        format!("Edit {name}"),
        Edit::SetFeatureKind {
            id: feature,
            kind: FeatureKind::Pipe(pipe.clone()),
        },
    );
    Some(document.carrying_owners(transaction, feature, fields))
}

pub fn change(model: &Model, feature: FeatureId, pipe: Pipe) -> Result<Transaction, PipeError> {
    let document = model.document();
    let transaction = edit(document, feature, pipe).ok_or(PipeError::Gone { feature })?;
    field::checked(document, transaction)
}

pub fn with_operation(pipe: &Pipe, operation: BodyOperation) -> Pipe {
    Pipe {
        operation,
        ..pipe.clone()
    }
}

pub fn with_shape(pipe: &Pipe, shape: PipeShape) -> Pipe {
    Pipe {
        shape,
        ..pipe.clone()
    }
}

pub fn hollowed(pipe: &Pipe, hollow: bool, unit: LengthUnit) -> Pipe {
    Pipe {
        wall: hollow.then(|| unit.default_length(DEFAULT_WALL)),
        ..pipe.clone()
    }
}

pub fn selected_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    pipe: &Pipe,
) -> Result<Transaction, PipeError> {
    let path = sweep_tools::selected_path(model, selection, feature, None)?;

    if path == pipe.path {
        return Err(SweepError::AlreadyFollowed.into());
    }
    change(
        model,
        feature,
        Pipe {
            path,
            ..pipe.clone()
        },
    )
}

pub fn path_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    pipe: &Pipe,
) -> Result<Transaction, PipeError> {
    let path = sweep_tools::toggled_path(model, selection, feature, (&pipe.path, None))?;
    change(
        model,
        feature,
        Pipe {
            path,
            ..pipe.clone()
        },
    )
}

pub fn cleared(pipe: &Pipe) -> Pipe {
    Pipe {
        path: SweepPath::Unchosen,
        ..pipe.clone()
    }
}
