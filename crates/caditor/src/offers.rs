use caditor_document::{AxisReference, DatumAxis, DatumPlane};

use crate::{
    datum_tools,
    model::Model,
    pattern_tools::{self, PatternSource},
    selection::Selection,
    shell_tools::{self, FaceSource},
    sketch_placement::{self, FaceChoice},
    units::LengthUnit,
};

#[derive(Debug, Clone, PartialEq)]
struct Basis {
    selection: Selection,
    revision: u64,
    evaluation: u64,
    unit: LengthUnit,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Offers {
    pub sketch_face: Option<FaceChoice>,
    pub model_axis: Option<AxisReference>,
    pub datum_plane: Result<DatumPlane, &'static str>,
    pub datum_axis: Result<DatumAxis, &'static str>,
    pub shell: Result<FaceSource, &'static str>,
    pub pattern: Result<PatternSource, &'static str>,
    pub described: Vec<String>,
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
            shell: shell_tools::selected_faces(model, selection),
            described: selection
                .iter()
                .map(|pickable| pickable.describe(document, evaluation))
                .collect(),
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
            selection: selection.clone(),
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
