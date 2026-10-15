use caditor_document::{
    BodyOperation, Document, Edit, Evaluation, FeatureId, FeatureKind, Pipe, SweepPath, Transaction,
};
use caditor_kernel::{EdgeName, PipeShape};
use caditor_sketch::EntityId;

use crate::{
    commands::Command,
    editing::{self, EditingCommand},
    field,
    last_values::{Remembered, Starts},
    model::{Action, Model},
    queued,
    reference_picking::{Picking, Slot},
    selection::{Pickable, Selection},
    sweep_tools::{self, PathPick},
    units::LengthUnit,
};

pub const TITLE: &str = "Pipe";
pub const DEFAULT_SIZE: f64 = 10.0;
pub const DEFAULT_BEND: f64 = 10.0;
pub const DEFAULT_WALL: f64 = 1.0;
const GONE: &str = "The feature no longer exists";

#[derive(Debug, Clone, PartialEq)]
pub struct PipeStart {
    pub path: PathPick,
}

pub fn source(model: &Model, selection: &Selection) -> Result<PipeStart, &'static str> {
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
        (None, None) => helix.map_or(PathPick::Unchosen, |helix| PathPick::Helix { helix }),
        (Some(_), Some(_)) => return Err(sweep_tools::CURVES_AND_EDGES),
        (Some((sketch, _)), None) => {
            if curves.iter().any(|(other, _)| other != sketch) {
                return Err(sweep_tools::SEVERAL_SKETCHES);
            }
            PathPick::Curves {
                sketch: *sketch,
                curves: curves.iter().map(|(_, curve)| *curve).collect(),
            }
        }
        (None, Some((body, _))) => {
            if edges.iter().any(|(other, _)| other != body) {
                return Err(sweep_tools::PATH_EDGES_OF_SEVERAL_BODIES);
            }
            PathPick::Edges {
                body: *body,
                edges: edges.iter().map(|(_, edge)| *edge).collect(),
            }
        }
    };
    Ok(PipeStart { path })
}

pub fn create(
    document: &Document,
    evaluation: &Evaluation,
    start: &PipeStart,
    starts: &Starts,
) -> Result<(Transaction, FeatureId), &'static str> {
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
        Err(reason) => vec![queued::refused(Command::Pipe, TITLE, reason)],
    }
}

pub fn edit(document: &Document, feature: FeatureId, pipe: Pipe) -> Option<Transaction> {
    let name = &document.feature(feature)?.name;
    let transaction = Transaction::single(
        format!("Edit {name}"),
        Edit::SetFeatureKind {
            id: feature,
            kind: FeatureKind::Pipe(pipe),
        },
    );
    Some(document.carrying_owners(transaction, feature, []))
}

pub fn change(model: &Model, feature: FeatureId, pipe: Pipe) -> Result<Transaction, String> {
    let document = model.document();
    let transaction = edit(document, feature, pipe).ok_or_else(|| GONE.to_owned())?;
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
) -> Result<Transaction, String> {
    let path = sweep_tools::selected_path(model, selection, feature, None)?;
    if path == pipe.path {
        return Err(sweep_tools::ALREADY_FOLLOWED.to_owned());
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
) -> Result<Transaction, String> {
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
