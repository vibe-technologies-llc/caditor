use caditor_document::{Feature, FeatureId, Mirror, MirrorOutcome, Transaction};
use egui::{Id, RichText, Ui};

use crate::{
    bodies, datum_tools,
    feature_fields::{self, Picker, Segment},
    icons,
    mirror_tools::{self, MirrorToolError},
    model::{Action, Model},
    offset_face_panel, pattern_tools, plane_selector,
    reference_picking::Slot,
    reference_rows::RowCache,
    selection::Selection,
    widgets,
};

pub const DESCRIPTION: &str = "Reflects the bodies across the plane, each image as a body of its \
                               own, joined to its original or in its place";
pub const FEATURES_DESCRIPTION: &str = "Reflects the chosen features across the plane onto the \
                                        body, following every edit to them";
pub const FACES_DESCRIPTION: &str = "Reflects the region the chosen faces bound across the plane, \
                                     cut where they bound a cavity and joined where they bound \
                                     material, following every edit to them";
pub const RESULT: &str = "Result";
pub const NEW_BODY: &str = "New body";
pub const ADD_TO_BODY: &str = "Add to body";
pub const REPLACE_BODY: &str = "Replace body";
const NEW_BODY_HOVER: &str =
    "Each image becomes a body of its own, and the originals stay as they are";
const ADD_TO_BODY_HOVER: &str = "Each image is joined to its original, making one body of the two";
const REPLACE_BODY_HOVER: &str =
    "Each image takes its original's place, and the originals are gone";

fn outcome_words(outcome: MirrorOutcome) -> (&'static str, &'static str) {
    match outcome {
        MirrorOutcome::NewBody => (NEW_BODY, NEW_BODY_HOVER),
        MirrorOutcome::AddToBody => (ADD_TO_BODY, ADD_TO_BODY_HOVER),
        MirrorOutcome::Replace => (REPLACE_BODY, REPLACE_BODY_HOVER),
    }
}
const PICK_HOVER: &str = "Mirror across the selected plane or flat face instead";
pub const BODIES: &str = "Bodies";
const ADD_BODIES_HOVER: &str = "Mirror the selected bodies too";
const STOP_MIRRORING_BODY: &str = "Stop mirroring this body";
const LAST_BODY_HOVER: &str = "The mirror needs a body: add another before leaving this one out";
pub const MIRRORS: &str = "Mirrors";
pub const WHOLE_BODY: &str = "The whole body";
pub const MIRROR_CHOSEN: &str = "Mirror the chosen features";
pub const MIRROR_SELECTED_FACES: &str = "Mirror the selected faces";
const MIRROR_FACES_HINT: &str = "Select every face around a pocket or boss of this body in the \
                                 view, such as a pocket's walls and floor, then mirror the region \
                                 they bound instead";
const STOP_MIRRORING_FACE: &str = "Stop mirroring this face";
const FACE_NOT_FOUND: &str = "A face of a body that has no shape yet";
const MIRROR_CHOSEN_HINT: &str = "Choose features of this body (extrusions, revolves, sweeps, \
                                  lofts, pipes, coils, holes, fillets, chamfers, shells, embosses \
                                  or primitives) above the mirror in the tree (Ctrl+click), then \
                                  mirror them instead of the whole body";

struct Panel<'a> {
    model: &'a Model,
    selection: &'a Selection,
    chosen: &'a [FeatureId],
    feature: &'a Feature,
    mirror: &'a Mirror,
    opened: bool,
    actions: &'a mut Vec<Action>,
}

impl Panel<'_> {
    fn id(&self) -> FeatureId {
        self.feature.id()
    }

    fn plane_rows(&mut self, ui: &mut Ui) {
        let document = self.model.document();
        let id = self.id();
        let picker = Picker {
            feature: id,
            slot: Slot::MirrorPlane,
            selected: feature_fields::offered_change(
                ui.ctx(),
                self.model,
                self.selection,
                (id, Slot::MirrorPlane),
                || mirror_tools::plane_change(self.model, self.selection, id, self.mirror),
            ),
            hover: PICK_HOVER,
        };

        let row = plane_selector::Row {
            caption: "Mirror across",
            id: Id::unique(("mirror-plane", id)),
            planes: datum_tools::listed_planes(document, id),
            current: Some(&self.mirror.plane),
            picker,
            remove: None,
        };

        plane_selector::row(
            ui,
            self.model,
            row,
            |reference| {
                mirror_tools::change(
                    self.model,
                    id,
                    Mirror {
                        plane: reference.clone(),
                        ..self.mirror.clone()
                    },
                )
            },
            self.actions,
        );
    }

    fn mirrors_rows(&mut self, ui: &mut Ui, cache: &mut RowCache) {
        let document = self.model.document();
        let mirrored = self.mirror.mirrored.clone();

        widgets::caption(ui, MIRRORS);
        if self.mirror.mirrors_faces() {
            self.face_rows(ui, cache);
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

    fn face_rows(&mut self, ui: &mut Ui, cache: &mut RowCache) {
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
        let mut previewed = Vec::new();

        for (index, text) in rows.iter().enumerate() {
            if index > 0 {
                ui.label("");
            }

            let row =
                widgets::removable_row_hovered(ui, widgets::muted(text, ui), STOP_MIRRORING_FACE);

            if row.hovered && self.opened {
                previewed = seen
                    .map(|seen| {
                        bodies::face_pickables(
                            self.mirror.body,
                            &seen.solid,
                            self.mirror.resolutions(&seen.solid).get(index),
                        )
                    })
                    .unwrap_or_default();
            }
            if row.removed {
                dropped = Some(index);
            }
            ui.end_row();
        }
        if !previewed.is_empty() {
            cache.preview(previewed);
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

    fn bodies_rows(&mut self, ui: &mut Ui) {
        let document = self.model.document();
        let bodies: Vec<FeatureId> = self.mirror.bodies().collect();
        let only = bodies.len() == 1;
        let mut dropped = None;

        for (index, body) in bodies.iter().enumerate() {
            if index == 0 {
                widgets::caption(ui, BODIES);
            } else {
                ui.label("");
            }
            match document.body_label(*body) {
                Some(name) => {
                    let hover = if only {
                        LAST_BODY_HOVER
                    } else {
                        STOP_MIRRORING_BODY
                    };

                    ui.add_enabled_ui(!only, |ui| {
                        if widgets::removable_row(ui, RichText::new(name), hover) {
                            dropped = Some(*body);
                        }
                    });
                }
                None => feature_fields::missing(ui, feature_fields::MISSING_BODY),
            }
            ui.end_row();
        }
        if let Some(body) = dropped {
            let change = mirror_tools::without_body(self.model, self.id(), self.mirror, body);

            self.apply(change);
        }
        ui.label("");

        let picker = Picker {
            feature: self.id(),
            slot: Slot::MirrorBodies,
            selected: feature_fields::offered_change(
                ui.ctx(),
                self.model,
                self.selection,
                (self.id(), Slot::MirrorBodies),
                || mirror_tools::added_bodies(self.model, self.selection, self.id(), self.mirror),
            ),
            hover: ADD_BODIES_HOVER,
        };

        ui.vertical(|ui| {
            feature_fields::reference_picker(ui, self.model, picker, self.actions);
        });
        ui.end_row();
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

    fn apply(&mut self, change: Result<Transaction, MirrorToolError>) {
        self.actions
            .push(feature_fields::applied(&self.feature.name, change));
    }

    fn result_row(&mut self, ui: &mut Ui) {
        if !self.mirror.mirrors_whole_body() {
            return;
        }

        let current = self.mirror.outcome;
        let segments = MirrorOutcome::ALL
            .into_iter()
            .map(|outcome| {
                let (label, hover) = outcome_words(outcome);
                Segment {
                    label,
                    hover,
                    change: (outcome != current).then(|| {
                        mirror_tools::change(
                            self.model,
                            self.id(),
                            self.mirror.clone().with_outcome(outcome),
                        )
                    }),
                }
            })
            .collect();
        let chosen = feature_fields::segmented_row(ui, RESULT, &self.feature.name, segments);

        self.actions.extend(chosen);
    }
}

pub fn show(
    ui: &mut Ui,
    model: &Model,
    (selection, chosen, opened): (&Selection, &[FeatureId], bool),
    cache: &mut RowCache,
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
        opened,
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
        panel.mirrors_rows(ui, cache);
        panel.result_row(ui);
        if mirror.mirrors_whole_body() {
            panel.bodies_rows(ui);
        } else {
            feature_fields::feature_row(ui, model.document(), "Body", mirror.body);
        }
    });
}
