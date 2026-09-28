mod geometry;
mod graph;
mod spline;
mod structure;
#[cfg(test)]
mod tests;
mod topology;
mod units;

use caditor_geometry::RigidTransform;
use caditor_kernel::Solid;

use crate::{
    part21::{SyntaxError, parse},
    read::{
        geometry::Geometry,
        graph::{Entity, Graph, Problem},
        structure::{MAX_DEPTH, MAX_INSTANCES as MAX_PLACEMENTS, Placements, Structure, Unplaced},
        topology::Topology,
        units::Units,
    },
};

const SOLID_KINDS: [&str; 2] = ["MANIFOLD_SOLID_BREP", "BREP_WITH_VOIDS"];
const UNIT_SLACK: f64 = 1e-9;

#[derive(Debug, Clone, PartialEq)]
pub struct StepSolid {
    pub name: String,
    pub solid: Solid,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct StepModel {
    pub solids: Vec<StepSolid>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReadError {
    #[error("it is not a STEP file")]
    NotStep,
    #[error("the file is damaged near line {0}")]
    Damaged(usize),
    #[error("it holds no solid bodies; caditor imports closed solids only")]
    NoSolids,
    #[error("{0}")]
    Unreadable(String),
}

pub fn read_step(text: &str) -> Result<StepModel, ReadError> {
    let exchange = parse(text).map_err(|error| match error {
        SyntaxError::NotStep => ReadError::NotStep,
        SyntaxError::Damaged { line } => ReadError::Damaged(line),
    })?;
    let graph = Graph::new(&exchange);
    let mut structure = Structure::read(&graph);
    let mut model = StepModel::default();
    let mut failures = Vec::new();
    let mut unnamed_units = false;
    let mut converted: Vec<f64> = Vec::new();
    let mut repaired = 0;
    let solids: Vec<Entity<'_>> = graph
        .entities()
        .filter(|entity| SOLID_KINDS.contains(&entity.kind()))
        .collect();
    let skipped = graph
        .entities()
        .filter(|entity| matches!(entity.kind(), "SHELL_BASED_SURFACE_MODEL" | "FACETED_BREP"))
        .count();
    let mut budget = MAX_PLACEMENTS;
    let mut unplaced = Vec::new();
    for (index, entity) in solids.iter().enumerate() {
        let representation = structure.representation_of(entity.id);
        let units = representation
            .map(|representation| structure.units_of(&graph, representation))
            .unwrap_or_default();
        unnamed_units |= !units.named;
        if units.named && (units.length - 1.0).abs() > UNIT_SLACK {
            converted.push(units.length);
        }
        let name = solid_name(entity, &structure, representation, index);
        let placements = match representation {
            Some(representation) => structure.placements(representation),
            None => Placements {
                transforms: vec![RigidTransform::IDENTITY],
                unplaced: None,
            },
        };
        if let Some(reason) = placements.unplaced {
            unplaced.push(unplaced_note(&name, reason));
            continue;
        }
        if placements.transforms.len() > budget {
            structure.truncated = true;
        }
        let transforms: Vec<RigidTransform> =
            placements.transforms.into_iter().take(budget).collect();
        if transforms.is_empty() {
            continue;
        }
        match build(&graph, units, entity.id) {
            Ok((solid, healed)) => {
                repaired += healed;
                let count = transforms.len();
                let mut misplaced = false;
                for (instance, placement) in transforms.into_iter().enumerate() {
                    let placed = if placement == RigidTransform::IDENTITY {
                        Ok(solid.clone())
                    } else {
                        solid.transformed(&placement)
                    };
                    let Ok(placed) = placed else {
                        misplaced = true;
                        continue;
                    };
                    let name = if count > 1 && instance > 0 {
                        format!("{name} {}", instance + 1)
                    } else {
                        name.clone()
                    };
                    budget = budget.saturating_sub(1);
                    model.solids.push(StepSolid {
                        name,
                        solid: placed,
                    });
                }
                if misplaced {
                    unplaced.push(format!(
                        "A copy of “{name}” was left out, because its placement in the assembly \
                         is not a rigid move."
                    ));
                }
            }
            Err(problem) => {
                log::warn!("could not import solid #{}: {problem}", entity.id);
                failures.push((name, problem));
            }
        }
    }
    for (name, problem) in &failures {
        model.notes.push(format!(
            "“{name}” could not be imported, because its entity {problem}."
        ));
    }
    model.notes.extend(unplaced.iter().cloned());
    if structure.truncated {
        model.notes.push(format!(
            "The file places parts more than {MAX_PLACEMENTS} times in all; only the first \
             {MAX_PLACEMENTS} copies were imported."
        ));
    }
    if skipped > 0 {
        model.notes.push(format!(
            "{} that {} not closed solids {} left out.",
            if skipped == 1 {
                "1 surface body".to_owned()
            } else {
                format!("{skipped} surface bodies")
            },
            if skipped == 1 { "is" } else { "are" },
            if skipped == 1 { "was" } else { "were" }
        ));
    }
    if repaired > 0 {
        model.notes.push(format!(
            "{} that did not quite meet {} moved onto {} faces.",
            if repaired == 1 {
                "1 edge or corner".to_owned()
            } else {
                format!("{repaired} edges and corners")
            },
            if repaired == 1 {
                "its faces was"
            } else {
                "their faces were"
            },
            if repaired == 1 { "its" } else { "their" }
        ));
    }
    converted.sort_by(f64::total_cmp);
    converted.dedup_by(|a, b| (*a - *b).abs() <= UNIT_SLACK * b.abs());
    if !model.solids.is_empty() {
        for scale in converted {
            model.notes.push(format!(
                "The file measures lengths in {}, so they were converted to millimetres.",
                unit_name(scale)
            ));
        }
    }
    if unnamed_units && !model.solids.is_empty() {
        model.notes.push(
            "The file does not say which unit it uses, so its numbers were read as millimetres."
                .to_owned(),
        );
    }
    if model.solids.is_empty() {
        return Err(
            match (failures.into_iter().next(), unplaced.into_iter().next()) {
                (Some((name, problem)), _) => ReadError::Unreadable(format!(
                    "“{name}” could not be rebuilt, because its entity {problem}"
                )),
                (None, Some(note)) => ReadError::Unreadable(note.trim_end_matches('.').to_owned()),
                (None, None) => ReadError::NoSolids,
            },
        );
    }
    Ok(model)
}

fn unit_name(millimetres: f64) -> String {
    const NAMED: [(f64, &str); 9] = [
        (0.001, "micrometres"),
        (0.0254, "thousandths of an inch"),
        (10.0, "centimetres"),
        (25.4, "inches"),
        (100.0, "decimetres"),
        (304.8, "feet"),
        (914.4, "yards"),
        (1_000.0, "metres"),
        (1_000_000.0, "kilometres"),
    ];
    NAMED
        .iter()
        .find(|(scale, _)| (scale - millimetres).abs() <= UNIT_SLACK * scale)
        .map_or_else(
            || format!("units of {millimetres} mm"),
            |(_, name)| (*name).to_owned(),
        )
}

fn unplaced_note(name: &str, reason: Unplaced) -> String {
    match reason {
        Unplaced::InsideItself => {
            format!("“{name}” was left out, because the assembly places it inside itself.")
        }
        Unplaced::TooDeep => format!(
            "“{name}” was left out, because the assembly nests it more than {MAX_DEPTH} levels \
             deep."
        ),
    }
}

fn build(graph: &Graph<'_>, units: Units, id: u64) -> Result<(Solid, usize), Problem> {
    let geometry = Geometry {
        graph: *graph,
        units,
    };
    Topology::new(&geometry).solid(id)
}

fn solid_name(
    entity: &Entity<'_>,
    structure: &Structure,
    representation: Option<u64>,
    index: usize,
) -> String {
    let own = entity
        .fields()
        .map(|fields| fields.text(0).trim().to_owned())
        .unwrap_or_default();
    if !own.is_empty() {
        return own;
    }
    representation
        .and_then(|representation| structure.name_of(representation))
        .map_or_else(|| format!("Body {}", index + 1), str::to_owned)
}
