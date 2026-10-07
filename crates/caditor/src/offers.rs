use caditor_document::{AxisReference, Datum, DatumAxis, DatumPoint, FeatureId};

use crate::{
    combine_tools::{self, BodyPair},
    datum_tools,
    mirror_tools::{self, MirrorSource},
    model::Model,
    move_tools,
    pattern_tools::{self, PatternSource},
    scale_tools,
    selection::Selection,
    shell_tools::{self, FaceSource},
    sketch_placement::{self, FaceChoice},
    units::LengthUnit,
};

pub const MAX_DESCRIBED: usize = 12;

#[derive(Debug, Clone, PartialEq)]
struct Basis {
    selection: u64,
    revision: u64,
    evaluation: u64,
    unit: LengthUnit,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Offers {
    pub sketch_face: Option<FaceChoice>,
    pub model_axis: Option<AxisReference>,
    pub datum_plane: Result<Datum, &'static str>,
    pub datum_axis: Result<DatumAxis, &'static str>,
    pub datum_point: Result<DatumPoint, &'static str>,
    pub shell: Result<FaceSource, &'static str>,
    pub combine: Result<BodyPair, &'static str>,
    pub movement: Result<FeatureId, &'static str>,
    pub mirror: Result<MirrorSource, &'static str>,
    pub scale: Result<FeatureId, &'static str>,
    pub pattern: Result<PatternSource, &'static str>,
    pub described: Vec<String>,
    pub selected: usize,
}

impl Offers {
    fn of(model: &Model, selection: &Selection) -> Self {
        let document = model.document();
        let evaluation = model.evaluation();
        let end = document.bar_index();
        let model_axis = selection
            .iter()
            .find_map(|pickable| datum_tools::axis_reference(model, pickable, end));
        Self {
            sketch_face: sketch_placement::selected_face(selection)
                .filter(|face| sketch_placement::is_flat(model, *face)),
            pattern: pattern_tools::source(model, selection, model_axis.as_ref()),
            model_axis,
            datum_plane: datum_tools::plane_from_selection(model, selection, end),
            datum_axis: datum_tools::axis_from_selection(model, selection, end),
            datum_point: datum_tools::point_from_selection(model, selection, end),
            shell: shell_tools::selected_faces(model, selection),
            combine: combine_tools::selected_bodies(model, selection),
            movement: move_tools::selected_body(model, selection),
            mirror: mirror_tools::source(model, selection),
            scale: scale_tools::selected_body(model, selection),
            described: selection
                .iter()
                .take(MAX_DESCRIBED)
                .map(|pickable| pickable.describe(document, evaluation))
                .collect(),
            selected: selection.len(),
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
    pub fn refresh(&mut self, model: &Model, selection: &Selection) -> &Offers {
        let basis = Basis {
            selection: selection.generation(),
            revision: model.revision(),
            evaluation: model.evaluation_generation(),
            unit: model.length_unit(),
        };
        let (_, offers) = match self.current.take() {
            Some((known, offers)) if known == basis => self.current.insert((known, offers)),
            Some(_) | None => {
                #[cfg(test)]
                {
                    self.computations += 1;
                }
                self.current.insert((basis, Offers::of(model, selection)))
            }
        };
        offers
    }

    #[cfg(test)]
    pub fn computations(&self) -> usize {
        self.computations
    }
}
