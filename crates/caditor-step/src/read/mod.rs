mod conform;
mod geometry;
mod graph;
mod loose;
mod offset;
mod presentation;
mod spline;
mod structure;
#[cfg(test)]
mod tests;
mod topology;
mod units;

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use caditor_geometry::Similarity;
use caditor_kernel::{Solid, check_interrupt};

pub use crate::read::presentation::FaceLook;
use crate::{
    part21::{Exchange, SyntaxError, parse},
    read::{
        geometry::{Geometry, MAX_WORK, Work},
        graph::{Entity, Graph, Problem},
        presentation::{Look, Presentation},
        structure::{MAX_DEPTH, MAX_INSTANCES as MAX_PLACEMENTS, Placements, Structure, Unplaced},
        topology::{Bending, Built, Healing, Lost, SolidShells, Topology, short, unreadable_faces},
        units::Units,
    },
};

const SOLID_KINDS: [&str; 3] = ["MANIFOLD_SOLID_BREP", "BREP_WITH_VOIDS", "FACETED_BREP"];
const UNIT_SLACK: f64 = 1e-9;
const TESSELLATED_KINDS: [&str; 5] = [
    "TESSELLATED_SOLID",
    "TESSELLATED_SHELL",
    "TESSELLATED_SURFACE_SET",
    "TRIANGULATED_SURFACE_SET",
    "COMPLEX_TRIANGULATED_SURFACE_SET",
];
const WIREFRAME_KINDS: [&str; 2] = ["GEOMETRIC_CURVE_SET", "GEOMETRIC_SET"];

#[derive(Debug, Clone, PartialEq)]
pub struct StepSolid {
    pub name: String,
    pub solid: Solid,
    pub colour: Option<[u8; 3]>,
    pub opacity: Option<u8>,
    pub layer: Option<String>,
    pub faces: Vec<FaceLook>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct StepModel {
    pub solids: Vec<StepSolid>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StepCopy {
    pub name: String,
    pub solid: Arc<Solid>,
    pub placement: Similarity,
    pub colour: Option<[u8; 3]>,
    pub opacity: Option<u8>,
    pub layer: Option<String>,
    pub faces: Vec<FaceLook>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct StepCopies {
    pub copies: Vec<StepCopy>,
    pub notes: Vec<String>,
}

struct Read<T> {
    solids: Vec<(String, Look, T)>,
    notes: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Held {
    pub schema: Option<String>,
    pub surface_bodies: usize,
    pub tessellated_shapes: usize,
    pub wireframes: usize,
}

impl std::fmt::Display for Held {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut found = Vec::new();
        if let Some(schema) = &self.schema {
            found.push(format!("its schema is {schema}"));
        }
        let counted = [
            (
                self.surface_bodies,
                "open surface body",
                "open surface bodies",
            ),
            (
                self.tessellated_shapes,
                "tessellated shape",
                "tessellated shapes",
            ),
            (self.wireframes, "wireframe", "wireframes"),
        ];
        let contents: Vec<String> = counted
            .into_iter()
            .filter(|(count, _, _)| *count > 0)
            .map(|(count, one, many)| format!("{count} {}", if count == 1 { one } else { many }))
            .collect();
        if !contents.is_empty() {
            found.push(format!("it holds {}", contents.join(", ")));
        }
        if found.is_empty() {
            return Ok(());
        }
        write!(formatter, " ({})", found.join(" and "))
    }
}

fn file_schema(exchange: &Exchange<'_>) -> Option<String> {
    let record = exchange
        .header()
        .find(|record| record.name() == "FILE_SCHEMA")?;
    let schemas = record.parameters().first()?.list()?;
    let first = schemas.first()?.text()?;
    let name = first.split_whitespace().next()?;
    Some(name.to_owned())
}

fn count_of(graph: &Graph<'_>, kinds: &[&str]) -> usize {
    graph
        .entities()
        .filter(|entity| kinds.contains(&entity.kind()))
        .count()
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReadError {
    #[error("it is not a STEP file")]
    NotStep,
    #[error("the file is damaged near line {0}")]
    Damaged(usize),
    #[error("it is larger than the 4 GiB caditor reads")]
    TooLarge,
    #[error("reading it was stopped")]
    Cancelled,
    #[error("it holds no solid bodies{0}; caditor imports closed solids only")]
    NoSolids(Held),
    #[error("“{name}” could not be rebuilt, because its entity #{entity} {reason}")]
    NotRebuilt {
        name: String,
        entity: u64,
        reason: String,
    },
    #[error("{}", .misplacement.note(.name).trim_end_matches('.'))]
    NotPlaced {
        name: String,
        misplacement: Misplacement,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Misplacement {
    InsideItself,
    Unreadable,
    TooDeep,
    SomeCopiesUnreadable,
    CopyUnplaceable,
}

impl Misplacement {
    pub fn note(self, name: &str) -> String {
        match self {
            Self::InsideItself => {
                format!("“{name}” was left out, because the assembly places it inside itself.")
            }
            Self::Unreadable => format!(
                "“{name}” was left out, because its placement in the assembly could not be read."
            ),
            Self::TooDeep => format!(
                "“{name}” was left out, because the assembly nests it more than {MAX_DEPTH} \
                 levels deep."
            ),
            Self::SomeCopiesUnreadable => format!(
                "Some copies of “{name}” were left out, because their placement in the assembly \
                 could not be read."
            ),
            Self::CopyUnplaceable => format!(
                "A copy of “{name}” was left out, because its placement in the assembly scales it \
                 beyond what caditor can model."
            ),
        }
    }
}

impl From<Unplaced> for Misplacement {
    fn from(reason: Unplaced) -> Self {
        match reason {
            Unplaced::InsideItself => Self::InsideItself,
            Unplaced::Unreadable => Self::Unreadable,
            Unplaced::TooDeep => Self::TooDeep,
        }
    }
}

pub fn read_step(text: &str) -> Result<StepModel, ReadError> {
    let read = read_placed(text, |solid, placement| {
        if placement == Similarity::IDENTITY {
            Some(Solid::clone(solid))
        } else {
            solid.mapped(&placement).ok()
        }
    })?;
    Ok(StepModel {
        solids: read
            .solids
            .into_iter()
            .map(|(name, look, solid)| StepSolid {
                name,
                solid,
                colour: look.colour,
                opacity: look.opacity,
                layer: look.layer,
                faces: look.faces,
            })
            .collect(),
        notes: read.notes,
    })
}

pub fn read_step_copies(text: &str) -> Result<StepCopies, ReadError> {
    let read = read_placed(text, |solid, placement| {
        Some((Arc::clone(solid), placement))
    })?;
    Ok(StepCopies {
        copies: read
            .solids
            .into_iter()
            .map(|(name, look, (solid, placement))| StepCopy {
                name,
                solid,
                placement,
                colour: look.colour,
                opacity: look.opacity,
                layer: look.layer,
                faces: look.faces,
            })
            .collect(),
        notes: read.notes,
    })
}

fn read_placed<T>(
    text: &str,
    mut place: impl FnMut(&Arc<Solid>, Similarity) -> Option<T>,
) -> Result<Read<T>, ReadError> {
    check_interrupt().map_err(|_| ReadError::Cancelled)?;
    let exchange = parse(text).map_err(|error| match error {
        SyntaxError::NotStep => ReadError::NotStep,
        SyntaxError::Damaged { line } => ReadError::Damaged(line),
        SyntaxError::TooLarge => ReadError::TooLarge,
    })?;
    let graph = Graph::new(&exchange);
    let presentation = Presentation::of(&graph);
    let mut structure = Structure::read(&graph);
    let mut model = Read {
        solids: Vec::new(),
        notes: Vec::new(),
    };
    let mut failures = Vec::new();
    let mut unnamed_units = false;
    let mut converted: Vec<f64> = Vec::new();
    let mut repaired = 0;
    let mut faceted_bodies = Vec::new();
    let mut bent_bodies = Vec::new();
    let mut unchecked_notes = Vec::new();
    let mut lost_notes = Vec::new();
    let encloses = |entity: &Entity<'_>| {
        entity.fields().is_ok_and(|fields| {
            fields.references(1).is_ok_and(|shells| {
                shells.into_iter().any(|shell| {
                    graph
                        .entity(shell)
                        .is_ok_and(|shell| shell.kind() == "CLOSED_SHELL")
                })
            })
        })
    };
    let solids: Vec<Entity<'_>> = graph
        .entities()
        .filter(|entity| {
            SOLID_KINDS.contains(&entity.kind())
                || (entity.kind() == "SHELL_BASED_SURFACE_MODEL" && encloses(entity))
        })
        .collect();
    let skipped = graph
        .entities()
        .filter(|entity| entity.kind() == "SHELL_BASED_SURFACE_MODEL" && !encloses(entity))
        .count();
    let mut solids_per_representation: BTreeMap<u64, usize> = BTreeMap::new();
    for entity in &solids {
        if let Some(representation) = structure.representation_of(entity.id) {
            *solids_per_representation.entry(representation).or_default() += 1;
        }
    }
    let mut budget = MAX_PLACEMENTS;
    let mut unplaced = Vec::new();
    let mut builder = Builder::new(graph);
    for (index, entity) in solids.iter().enumerate() {
        check_interrupt().map_err(|_| ReadError::Cancelled)?;
        let representation = structure.representation_of(entity.id);
        let units = representation
            .map(|representation| structure.units_of(&graph, representation))
            .unwrap_or_default();
        unnamed_units |= !units.named;
        if units.named && (units.length - 1.0).abs() > UNIT_SLACK {
            converted.push(units.length);
        }
        let siblings = representation.map_or(1, |representation| {
            solids_per_representation
                .get(&representation)
                .copied()
                .unwrap_or(1)
        });
        let name = solid_name(entity, &structure, representation, index, siblings);
        let placements = match representation {
            Some(representation) => structure.placements(representation),
            None => Placements::at_origin(),
        };
        if let Some(reason) = placements.unplaced {
            unplaced.push((name, reason.into()));
            continue;
        }
        if placements.left_out {
            unplaced.push((name.clone(), Misplacement::SomeCopiesUnreadable));
        }
        if placements.transforms.len() > budget {
            structure.truncated = true;
        }
        let transforms: Vec<(Similarity, Option<String>)> = placements
            .transforms
            .into_iter()
            .zip(
                placements
                    .occurrences
                    .into_iter()
                    .chain(std::iter::repeat(None)),
            )
            .take(budget)
            .collect();
        if transforms.is_empty() {
            continue;
        }
        let named_occurrences: BTreeSet<&str> = transforms
            .iter()
            .filter_map(|(_, occurrence)| occurrence.as_deref())
            .collect();
        let occurrences_name_each =
            transforms.len() > 1 && named_occurrences.len() == transforms.len();
        match builder.build(units, entity.id) {
            Ok(Built {
                solid,
                healed,
                unchecked,
                faceted,
                bent,
                faces,
                lost,
            }) => {
                match lost {
                    Some(lost) => lost_notes.push(lost_note(&name, &lost)),
                    None if faceted => faceted_bodies.push(name.clone()),
                    None => {}
                }
                if !faceted {
                    repaired += healed;
                }
                if let Some(bending) = bent.filter(|bending| bending.faces > 0) {
                    bent_bodies.push((name.clone(), bending, units.precision));
                }
                if let Some(faces) = unchecked {
                    unchecked_notes.push(unchecked_note(&name, faces));
                }
                let count = transforms.len();
                let mut look = presentation.of_solid(&graph, entity);
                look.faces = presentation.of_faces(&look, &faces);
                let mut misplaced = false;
                let solid = Arc::new(solid);
                for (instance, (placement, occurrence)) in transforms.into_iter().enumerate() {
                    let Some(placed) = place(&solid, placement) else {
                        misplaced = true;
                        continue;
                    };
                    let name = match occurrence {
                        Some(occurrence) if occurrences_name_each => occurrence,
                        _ if count > 1 && instance > 0 => format!("{name} {}", instance + 1),
                        _ => name.clone(),
                    };
                    budget = budget.saturating_sub(1);
                    model.solids.push((name, look.clone(), placed));
                }
                if misplaced {
                    unplaced.push((name.clone(), Misplacement::CopyUnplaceable));
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
    model.notes.extend(lost_notes);
    model.notes.extend(
        unplaced
            .iter()
            .map(|(name, misplacement)| misplacement.note(name)),
    );
    model
        .notes
        .extend(faceted_bodies.iter().map(|name| faceted_note(name)));
    model.notes.extend(
        bent_bodies
            .iter()
            .map(|(name, bending, precision)| bent_note(name, *bending, *precision)),
    );
    model.notes.extend(unchecked_notes);
    model.notes.extend(damage_notes(&exchange));
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
                (Some((name, problem)), _) => ReadError::NotRebuilt {
                    name,
                    entity: problem.entity,
                    reason: problem.reason,
                },
                (None, Some((name, misplacement))) => ReadError::NotPlaced { name, misplacement },
                (None, None) => ReadError::NoSolids(Held {
                    schema: file_schema(&exchange),
                    surface_bodies: skipped,
                    tessellated_shapes: count_of(&graph, &TESSELLATED_KINDS),
                    wireframes: count_of(&graph, &WIREFRAME_KINDS),
                }),
            },
        );
    }
    Ok(model)
}

fn damage_notes(exchange: &Exchange) -> Vec<String> {
    let mut notes = Vec::new();
    if let Some(first) = exchange.unreadable.first() {
        notes.push(match exchange.unreadable.len() {
            1 => format!(
                "An entry of the file near line {first} could not be read and was left out."
            ),
            count => format!(
                "{count} entries of the file, the first near line {first}, could not be read and \
                 were left out."
            ),
        });
    }
    if exchange.header_damaged {
        notes.push("The header of the file is damaged and was ignored.".to_owned());
    }
    if exchange.cut_short {
        notes.push(
            "The file ends in the middle of its data, so it is cut short (an interrupted download \
             or copy leaves it so); everything before the end was read."
                .to_owned(),
        );
    } else if exchange.trailer_missing {
        notes.push(
            "The file ends without its closing line, so it may be cut short; everything before \
             the end was read."
                .to_owned(),
        );
    }
    if let Some(first) = exchange.repeated.first() {
        notes.push(match exchange.repeated.len() {
            1 => format!(
                "The file defines entity #{first} more than once; its first definition was used."
            ),
            count => format!(
                "The file defines {count} entities, such as #{first}, more than once; their first \
                 definitions were used."
            ),
        });
    }
    notes
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

fn faceted_note(name: &str) -> String {
    format!(
        "“{name}” was imported as flat facets, because its faces meet only within the file's \
         precision and not within the 0.000001 mm caditor needs; its curved faces are \
         approximated by flat ones, so they cannot be filleted or measured as curves."
    )
}

fn lost_note(name: &str, lost: &Lost) -> String {
    let reason = format!("its entity {}", lost.problem);
    let faces: Vec<String> = lost.faces.iter().map(|face| format!("#{face}")).collect();
    let listed = match faces.as_slice() {
        [] => String::new(),
        [only] => only.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    };
    let plural = faces.len() > 1;
    if lost.voids + lost.lumps == 0 {
        return format!(
            "“{name}” was imported without its {} {listed}, because {reason}; the {} left \
             {} closed with flat facets, so its curved faces are approximated by flat ones.",
            if plural { "faces" } else { "face" },
            if plural { "holes they" } else { "hole it" },
            if plural { "were" } else { "was" },
        );
    }
    let mut parts = Vec::new();
    for (count, one, many) in [
        (lost.voids, "hollow", "hollows"),
        (lost.lumps, "separate piece", "separate pieces"),
    ] {
        match count {
            0 => {}
            1 => parts.push(format!("1 {one}")),
            count => parts.push(format!("{count} {many}")),
        }
    }
    format!(
        "“{name}” was imported without {} of it, holding its {} {listed} that could not be read, \
         because {reason}.",
        parts.join(" and "),
        if plural { "faces" } else { "face" },
    )
}

fn bent_note(name: &str, bending: Bending, precision: Option<f64>) -> String {
    let faces = if bending.faces == 1 {
        "1 curved face was".to_owned()
    } else {
        format!("{} curved faces were", bending.faces)
    };
    let within = precision.map_or_else(String::new, |precision| {
        format!(", within the file's precision of {} mm", short(precision))
    });
    format!(
        "In “{name}”, {faces} bent by up to {} mm so that they meet their neighbours exactly{within}.",
        short(bending.farthest)
    )
}

fn unchecked_note(name: &str, [first, second]: [u64; 2]) -> String {
    let what = if first == second {
        format!("the edges of its face #{first} cross each other")
    } else {
        format!("its faces #{first} and #{second} cross each other")
    };
    format!(
        "“{name}” was imported, but whether {what} could not be checked, so features built on it \
         may fail."
    )
}

type Builds = BTreeMap<(SolidShells, [u64; 3]), (u64, Result<Built, Problem>)>;

struct Builder<'a> {
    graph: Graph<'a>,
    work: Work,
    geometries: Vec<Geometry<'a>>,
    builds: Builds,
}

impl<'a> Builder<'a> {
    fn new(graph: Graph<'a>) -> Self {
        Self {
            graph,
            work: Work::new(MAX_WORK),
            geometries: Vec::new(),
            builds: BTreeMap::new(),
        }
    }

    fn build(&mut self, units: Units, id: u64) -> Result<Built, Problem> {
        let shells = SolidShells::of(&self.graph, id)?;
        let key = (
            shells,
            [
                units.length.to_bits(),
                units.angle.to_bits(),
                units.uncertainty().to_bits(),
            ],
        );
        if let Some((first, built)) = self.builds.get(&key) {
            let first = *first;
            return built.clone().map_err(|problem| {
                if problem.entity == first {
                    Problem::new(id, problem.reason)
                } else {
                    problem
                }
            });
        }
        self.work.charge(id)?;
        let position = match self
            .geometries
            .iter()
            .position(|geometry| geometry.units == units)
        {
            Some(position) => position,
            None => {
                self.geometries
                    .push(Geometry::new(self.graph, units, self.work.clone()));
                self.geometries.len() - 1
            }
        };
        let geometry = self
            .geometries
            .get(position)
            .ok_or_else(|| Problem::new(id, "could not be read"))?;
        let built = healed(geometry, id, &key.0)
            .or_else(|problem| without_unreadable(geometry, id, &key.0, problem));
        self.builds.insert(key, (id, built.clone()));
        built
    }
}

fn healed(geometry: &Geometry<'_>, id: u64, shells: &SolidShells) -> Result<Built, Problem> {
    Topology::new(geometry, Healing::Exact)
        .solid(id, shells)
        .or_else(|problem| {
            Topology::new(geometry, Healing::Extended)
                .solid(id, shells)
                .map_err(|_| problem)
        })
        .or_else(|problem| match geometry.units.precision {
            Some(_) => Topology::new(geometry, Healing::Bent)
                .solid(id, shells)
                .or_else(|_| Topology::new(geometry, Healing::Faceted).solid(id, shells))
                .map_err(|_| problem),
            None => Err(problem),
        })
}

fn without_unreadable(
    geometry: &Geometry<'_>,
    id: u64,
    shells: &SolidShells,
    problem: Problem,
) -> Result<Built, Problem> {
    let unreadable = unreadable_faces(geometry, shells);
    let Some(first) = unreadable.first() else {
        return Err(problem);
    };
    let outer = shells.outer();
    let dropped: BTreeSet<u64> = unreadable
        .iter()
        .map(|face| face.shell)
        .filter(|shell| *shell != outer)
        .collect();
    let kept = shells.without(&dropped);
    let lost_faces: BTreeSet<u64> = unreadable
        .iter()
        .filter(|face| face.shell == outer)
        .map(|face| face.face)
        .collect();
    let built = if lost_faces.is_empty() {
        healed(geometry, id, &kept)
    } else {
        Topology::new(geometry, Healing::Faceted)
            .leaving_out(lost_faces.clone())
            .solid(id, &kept)
    };
    let Ok(mut built) = built else {
        return Err(problem);
    };
    let named = unreadable
        .iter()
        .find(|face| lost_faces.is_empty() || face.shell == outer)
        .unwrap_or(first);
    let voids = dropped
        .iter()
        .filter(|shell| shells.is_void(**shell))
        .count();
    built.lost = Some(Lost {
        faces: if lost_faces.is_empty() {
            unreadable.iter().map(|face| face.face).collect()
        } else {
            lost_faces.into_iter().collect()
        },
        problem: named.problem.clone(),
        voids,
        lumps: dropped.len() - voids,
    });
    Ok(built)
}

fn solid_name(
    entity: &Entity<'_>,
    structure: &Structure,
    representation: Option<u64>,
    index: usize,
    siblings: usize,
) -> String {
    let own = entity
        .fields()
        .ok()
        .and_then(|fields| fields.name(0))
        .unwrap_or_default();
    let product = representation.and_then(|representation| structure.name_of(representation));
    match product {
        Some(product) if own.is_empty() || siblings <= 1 => product.to_owned(),
        _ if !own.is_empty() => own,
        _ => format!("Body {}", index + 1),
    }
}
