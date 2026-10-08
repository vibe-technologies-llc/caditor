use caditor_document::{AxisReference, Datum, DatumAxis, FeatureId};

use crate::{
    combine_tools::{self, BodyPair},
    datum_tools, measure,
    mirror_tools::{self, MirrorSource},
    model::Model,
    move_tools,
    pattern_tools::{self, PatternSource},
    scale_tools,
    selection::{Pickable, Selection},
    shell_tools::{self, FaceSource},
    sketch_placement::{self, SketchTarget},
    split_tools::{self, SplitSource},
    units::LengthUnit,
};

pub const MAX_DESCRIBED: usize = 12;

#[derive(Debug, Clone, PartialEq)]
struct Basis {
    selection: u64,
    tree: Vec<FeatureId>,
    rows: Vec<FeatureId>,
    revision: u64,
    evaluation: u64,
    unit: LengthUnit,
    meshing: bool,
}

pub type Chosen<'a> = (&'a [FeatureId], &'a [FeatureId]);

#[derive(Debug, Clone, PartialEq)]
pub struct Offers {
    pub sketch_target: Result<SketchTarget, &'static str>,
    pub model_axes: Vec<(Pickable, AxisReference)>,
    pub datum_plane: Result<Datum, &'static str>,
    pub datum_axis: Result<DatumAxis, &'static str>,
    pub datum_point: Result<Datum, &'static str>,
    pub shell: Result<FaceSource, &'static str>,
    pub combine: Result<BodyPair, &'static str>,
    pub movement: Result<FeatureId, &'static str>,
    pub mirror: Result<MirrorSource, &'static str>,
    pub split: Result<SplitSource, &'static str>,
    pub scale: Result<FeatureId, &'static str>,
    pub pattern: Result<PatternSource, &'static str>,
    pub described: Vec<String>,
    pub selected: usize,
    pub size: Option<String>,
}

fn size_of(model: &Model, selection: &Selection, tree: &[FeatureId]) -> Option<String> {
    match (selection.iter().collect::<Vec<_>>().as_slice(), tree) {
        ([only], _) => measure::size_text(model, *only),
        ([], [body]) => measure::body_size_text(model, *body),
        _ => None,
    }
}

impl Offers {
    fn of(model: &Model, selection: &Selection, (tree, rows): Chosen<'_>) -> Self {
        let document = model.document();
        let evaluation = model.evaluation();
        let end = document.bar_index();
        let model_axes = selection
            .in_pick_order()
            .into_iter()
            .filter_map(|pickable| {
                Some((pickable, datum_tools::axis_reference(model, pickable, end)?))
            })
            .collect();
        Self {
            sketch_target: sketch_placement::sketch_target(model, selection),
            pattern: pattern_tools::source(model, selection, tree, rows),
            model_axes,
            datum_plane: datum_tools::plane_from_selection(model, selection, end),
            datum_axis: datum_tools::axis_from_selection(model, selection, end),
            datum_point: datum_tools::point_from_selection(model, selection, end),
            shell: shell_tools::selected_faces(model, selection),
            combine: combine_tools::selected_bodies(model, selection, tree),
            movement: move_tools::selected_body(model, selection, tree),
            mirror: mirror_tools::source(model, selection, tree),
            split: split_tools::source(model, selection, tree),
            scale: scale_tools::selected_body(model, selection, tree),
            described: selection
                .iter()
                .take(MAX_DESCRIBED)
                .map(|pickable| pickable.describe(document, evaluation))
                .collect(),
            selected: selection.len(),
            size: size_of(model, selection, tree),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct SelectionOffers {
    current: Option<(Basis, Offers)>,
    #[cfg(test)]
    computations: usize,
}

impl SelectionOffers {
    pub fn refresh(&mut self, model: &Model, selection: &Selection, chosen: Chosen<'_>) -> &Offers {
        let (tree, rows) = chosen;
        let basis = Basis {
            selection: selection.generation(),
            tree: tree.to_vec(),
            rows: rows.to_vec(),
            revision: model.revision(),
            evaluation: model.evaluation_generation(),
            unit: model.length_unit(),
            meshing: model.bodies_pending(),
        };
        let (_, offers) = match self.current.take() {
            Some((known, offers)) if known == basis => self.current.insert((known, offers)),
            Some(_) | None => {
                #[cfg(test)]
                {
                    self.computations += 1;
                }
                self.current
                    .insert((basis, Offers::of(model, selection, chosen)))
            }
        };
        offers
    }

    #[cfg(test)]
    pub fn computations(&self) -> usize {
        self.computations
    }
}
