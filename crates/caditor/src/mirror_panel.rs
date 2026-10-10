use caditor_document::{Feature, FeatureId, Mirror, Transaction, capitalized, describe_plane};
use egui::{Id, Ui};

use crate::{
    datum_tools,
    feature_fields::{self, Choice, Picker},
    icons, mirror_tools,
    model::{Action, Model},
    offset_face_panel, pattern_tools,
    reference_picking::Slot,
    selection::Selection,
    widgets,
};

pub const DESCRIPTION: &str = "Reflects the body across the plane, joined to the original or in \
                               its place";
pub const FEATURES_DESCRIPTION: &str = "Reflects the chosen features across the plane onto the \
                                        body, following every edit to them";
pub const FACES_DESCRIPTION: &str = "Reflects the region the chosen faces bound across the plane, \
                                     cut where they bound a cavity and joined where they bound \
                                     material, following every edit to them";
pub const KEEP_ORIGINAL: &str = "Keep the original";
const PICK_HOVER: &str = "Mirror across the selected plane or flat face instead";
pub const MIRRORS: &str = "Mirrors";
pub const WHOLE_BODY: &str = "The whole body";
pub const MIRROR_CHOSEN: &str = "Mirror the chosen features";
pub const MIRROR_SELECTED_FACES: &str = "Mirror the selected faces";
const MIRROR_FACES_HINT: &str = "Select every face around a pocket or boss of this body in the \
                                 view, such as a pocket's walls and floor, then mirror the region \
                                 they bound instead";
const STOP_MIRRORING_FACE: &str = "Stop mirroring this face";
const FACE_NOT_FOUND: &str = "A face of a body that has no shape yet";
const MIRROR_CHOSEN_HINT: &str = "Choose extrusions, revolves, holes or primitives of this body above the \
                                  mirror in the tree (Ctrl+click), then mirror them instead of \
                                  the whole body";

struct Panel<'a> {
    model: &'a Model,
    selection: &'a Selection,
    chosen: &'a [FeatureId],
    feature: &'a Feature,
    mirror: &'a Mirror,
    actions: &'a mut Vec<Action>,
}

impl Panel<'_> {
    fn id(&self) -> FeatureId {
        self.feature.id()
    }

    fn change(&self, mirror: Mirror) -> Result<Action, String> {
        mirror_tools::change(self.model, self.id(), mirror).map(Action::Apply)
    }

    fn plane_rows(&mut self, ui: &mut Ui) {
        let document = self.model.document();
        let current = capitalized(&describe_plane(document, &self.mirror.plane));
        widgets::caption(ui, "Mirror across");
        let chosen =
            feature_fields::combo(ui, Id::new(("mirror-plane", self.id())), current, || {
                datum_tools::listed_planes(document, self.id())
                    .into_iter()
                    .map(|reference| Choice {
                        label: capitalized(&describe_plane(document, &reference)),
                        selected: self.mirror.plane == reference,
                        change: self.change(Mirror {
                            plane: reference,
                            ..self.mirror.clone()
                        }),
                    })
                    .collect()
            });
        ui.end_row();
        self.actions.extend(chosen);
        ui.label("");
        let picker = Picker {
            feature: self.id(),
            slot: Slot::MirrorPlane,
            selected: feature_fields::offered_change(
                ui.ctx(),
                self.model,
                self.selection,
                (self.id(), Slot::MirrorPlane),
                || mirror_tools::plane_change(self.model, self.selection, self.id(), self.mirror),
            ),
            hover: PICK_HOVER,
        };
        ui.vertical(|ui| {
            feature_fields::reference_picker(ui, self.model, picker, self.actions);
        });
        ui.end_row();
    }

    fn mirrors_rows(&mut self, ui: &mut Ui) {
        let document = self.model.document();
        let mirrored = self.mirror.mirrored.clone();
        widgets::caption(ui, MIRRORS);
        if self.mirror.mirrors_faces() {
            self.face_rows(ui);
        } else if mirrored.is_empty() {
            ui.label(WHOLE_BODY);
            ui.end_row();
        }
        for (index, feature) in mirrored.iter().enumerate() {
            if index > 0 {
                ui.label("");
            }
            let name = feature_fields::feature_name(document, *feature);
            let hover = format!("Stop mirroring {}", name.unwrap_or("the missing feature"));
            let mut dropped = false;
            ui.horizontal(|ui| {
                match name {
                    Some(name) => {
                        ui.label(name);
                    }
                    None => feature_fields::missing(ui, "Missing feature"),
                }
                dropped = widgets::icon_button(ui, icons::REMOVE, &hover).clicked();
            });
            ui.end_row();
            if dropped {
                let kept = mirrored
                    .iter()
                    .copied()
                    .filter(|kept| kept != feature)
                    .collect();
                let change = mirror_tools::mirroring(self.model, self.id(), self.mirror, kept);
                self.apply(change);
            }
        }
        let offered = pattern_tools::repeatable(document, self.chosen)
            .filter(|(body, chosen)| *body == self.mirror.body && *chosen != mirrored);
        ui.label("");
        let button = widgets::small_button(ui, icons::ADD, MIRROR_CHOSEN);
        let hover = match &offered {
            Some((body, chosen)) => format!(
                "Mirror {} instead of {}",
                pattern_tools::subject(document, *body, chosen),
                self.subject()
            ),
            None => MIRROR_CHOSEN_HINT.to_owned(),
        };
        let clicked = ui
            .add_enabled(offered.is_some(), button)
            .on_hover_text(&hover)
            .on_disabled_hover_text(&hover)
            .clicked();
        ui.end_row();
        if clicked && let Some((_, chosen)) = offered {
            let change = mirror_tools::mirroring(self.model, self.id(), self.mirror, chosen);
            self.apply(change);
        }
        self.selected_faces_row(ui);
    }

    fn face_rows(&mut self, ui: &mut Ui) {
        let document = self.model.document();
        let seen = self
            .model
            .evaluation()
            .body_result_seen_by(self.id(), self.mirror.body);
        let rows: Vec<String> = match seen {
            Some(seen) => self
                .mirror
                .resolutions(&seen.solid)
                .iter()
                .map(|resolution| offset_face_panel::face_row(document, seen, resolution))
                .collect(),
            None => vec![FACE_NOT_FOUND.to_owned(); self.mirror.faces.len()],
        };
        let mut dropped = None;
        for (index, text) in rows.iter().enumerate() {
            if index > 0 {
                ui.label("");
            }
            if widgets::removable_row(ui, widgets::muted(text, ui), STOP_MIRRORING_FACE) {
                dropped = Some(index);
            }
            ui.end_row();
        }
        if let Some(index) = dropped {
            let kept = self
                .mirror
                .faces
                .iter()
                .enumerate()
                .filter(|(kept, _)| *kept != index)
                .map(|(_, face)| face.clone())
                .collect();
            let change = mirror_tools::mirroring_faces(self.model, self.id(), self.mirror, kept);
            self.apply(change);
        }
    }

    fn selected_faces_row(&mut self, ui: &mut Ui) {
        let offered =
            mirror_tools::selected_faces(self.model, self.selection, self.id(), self.mirror);
        ui.label("");
        let button = widgets::small_button(ui, icons::ADD, MIRROR_SELECTED_FACES);
        let hover = match &offered {
            Ok(faces) => format!(
                "Mirror the region the {} selected faces bound instead of {}",
                faces.len(),
                self.subject()
            ),
            Err(reason) => format!("{MIRROR_FACES_HINT}. {reason}."),
        };
        let clicked = ui
            .add_enabled(offered.is_ok(), button)
            .on_hover_text(&hover)
            .on_disabled_hover_text(&hover)
            .clicked();
        ui.end_row();
        if clicked && let Ok(faces) = offered {
            let change = mirror_tools::mirroring_faces(self.model, self.id(), self.mirror, faces);
            self.apply(change);
        }
    }

    fn subject(&self) -> String {
        if self.mirror.mirrors_faces() {
            format!("{} chosen faces", self.mirror.faces.len())
        } else {
            pattern_tools::subject(
                self.model.document(),
                self.mirror.body,
                &self.mirror.mirrored,
            )
        }
    }

    fn apply(&mut self, change: Result<Transaction, String>) {
        self.actions
            .push(feature_fields::applied(&self.feature.name, change));
    }

    fn keep_row(&mut self, ui: &mut Ui) {
        if !self.mirror.mirrors_whole_body() {
            return;
        }
        if let Some(keep_original) =
            feature_fields::reverse_row(ui, KEEP_ORIGINAL, self.mirror.keep_original)
        {
            let change = mirror_tools::change(
                self.model,
                self.id(),
                Mirror {
                    keep_original,
                    ..self.mirror.clone()
                },
            );
            self.apply(change);
        }
    }
}

pub fn show(
    ui: &mut Ui,
    model: &Model,
    (selection, chosen): (&Selection, &[FeatureId]),
    actions: &mut Vec<Action>,
    feature: &Feature,
    mirror: &Mirror,
) {
    let mut panel = Panel {
        model,
        selection,
        chosen,
        feature,
        mirror,
        actions,
    };
    widgets::properties(ui, ("mirror-properties", feature.id()), |ui| {
        let description = if mirror.mirrors_features() {
            FEATURES_DESCRIPTION
        } else if mirror.mirrors_faces() {
            FACES_DESCRIPTION
        } else {
            DESCRIPTION
        };
        feature_fields::description_row(ui, description);
        panel.plane_rows(ui);
        panel.mirrors_rows(ui);
        panel.keep_row(ui);
        feature_fields::feature_row(ui, model.document(), "Body", mirror.body);
    });
}
