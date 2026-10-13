use caditor_document::{Feature, FeatureId, ImagePlane, ImageSize, ReferenceImage};
use caditor_expression::{Dimension, Expression};
use egui::{Id, Label, Ui};

use crate::{
    datum_tools,
    editing::EditingCommand,
    feature_fields::{self, Picker, Quantity, Rule, Segment},
    icons, image_tools,
    model::{Action, Model},
    plane_selector,
    reference_picking::{Picking, Slot},
    selection::Selection,
    widgets::{self, Tone},
};

pub const OPACITY_STEPS: [u8; 5] = [20, 40, 60, 80, 100];
pub const CALIBRATE: &str = "Calibrate by two points";
pub const CALIBRATE_AGAIN: &str = "Calibrate again";
pub const USE_WIDTH: &str = "Size by width";
pub const WIDTH: &str = "Width";
pub const ANGLE: &str = "Angle";
pub const DISTANCE: &str = "Distance between the points";
pub const FLIP: &str = "Flip left to right";
pub const LOCKED: &str = "Locked, so clicks pass through to sketching";
pub const BEHIND: &str = "Behind bodies";
pub const IN_FRONT: &str = "In front of bodies";
pub const PREPARING: &str = "Preparing the picture for showing…";
const PICK_HOVER: &str = "Place it on the selected plane or flat face instead";
const CALIBRATE_HOVER: &str = "Click two points on the picture, then type the real distance \
                               between them, and the picture is scaled to match";
const USE_WIDTH_HOVER: &str = "Size the picture by its width instead of the two points";
const BEHIND_HOVER: &str = "Bodies in front of the picture hide it";
const IN_FRONT_HOVER: &str = "The picture shows over every body";
const LOCKED_HOVER: &str = "The picture cannot be clicked or selected in the view, so sketch tools \
                            and selection reach what lies on and behind it";

pub fn nearest_opacity_step(percent: u8) -> u8 {
    OPACITY_STEPS
        .into_iter()
        .min_by_key(|step| step.abs_diff(percent))
        .unwrap_or(percent)
}

fn opacity_label(percent: u8) -> &'static str {
    match percent {
        20 => "20%",
        40 => "40%",
        60 => "60%",
        80 => "80%",
        _ => "100%",
    }
}

fn opacity_hover(percent: u8) -> &'static str {
    match percent {
        100 => "Show the picture solid",
        _ => "Show the picture see-through, so the sketch and bodies show through it",
    }
}

struct Panel<'a> {
    model: &'a Model,
    selection: &'a Selection,
    feature: &'a Feature,
    image: &'a ReferenceImage,
    actions: &'a mut Vec<Action>,
}

impl Panel<'_> {
    fn id(&self) -> FeatureId {
        self.feature.id()
    }

    fn change(&self, image: ReferenceImage) -> Result<caditor_document::Transaction, String> {
        image_tools::change(self.model, self.id(), image)
    }

    fn apply(&mut self, image: ReferenceImage) {
        let change = self.change(image);
        self.actions
            .push(feature_fields::applied(&self.feature.name, change));
    }

    fn description(&mut self, ui: &mut Ui) {
        let picture = &self.image.picture;
        let size = picture.size();
        let text = format!(
            "A {} picture, {} by {} pixels, to trace over",
            picture.format().name(),
            size.width,
            size.height
        );
        feature_fields::description_row(ui, &text);
        let digest = picture.digest();
        let textures = self.model.pictures();
        if let Some(reason) = textures.failure(digest) {
            ui.label("");
            widgets::callout(ui, Tone::Error, |ui| {
                ui.add(
                    Label::new(format!(
                        "The picture cannot be shown: {reason}. Insert it again from its file."
                    ))
                    .wrap(),
                );
            });
            ui.end_row();
        } else if textures.is_decoding(digest) {
            feature_fields::description_row(ui, PREPARING);
        }
    }

    fn plane_rows(&mut self, ui: &mut Ui) {
        if let ImagePlane::Sketch(sketch) = &self.image.plane {
            let name = feature_fields::feature_name(self.model.document(), *sketch)
                .unwrap_or(feature_fields::MISSING_SKETCH);
            feature_fields::description_row(ui, &format!("It lies on the plane of “{name}”."));
        }
        let picker = Picker {
            feature: self.id(),
            slot: Slot::ImagePlace,
            selected: feature_fields::offered_change(
                ui.ctx(),
                self.model,
                self.selection,
                (self.id(), Slot::ImagePlace),
                || image_tools::place_change(self.model, self.selection, self.id(), self.image),
            ),
            hover: PICK_HOVER,
        };
        let row = plane_selector::Row {
            caption: "Placed on",
            id: Id::new(("image-plane", self.id())),
            planes: datum_tools::listed_planes(self.model.document(), self.id()),
            current: self.image.plane.reference(),
            picker,
            remove: None,
        };
        let mut actions = Vec::new();
        plane_selector::row(
            ui,
            self.model,
            row,
            |plane| image_tools::placed_on(self.model, self.id(), self.image, plane),
            &mut actions,
        );
        self.actions.extend(actions);
    }

    fn value_row(
        &mut self,
        ui: &mut Ui,
        caption: &str,
        (key, expression): (&'static str, &Expression),
        (dimension, rule): (Dimension, Rule),
        changed: impl Fn(Expression) -> ReferenceImage,
    ) {
        let quantity = Quantity {
            feature: self.id(),
            id: Id::new(("image-field", self.id(), key)),
            expression,
            dimension,
            rule,
        };
        let drafting =
            feature_fields::expression_row_drafting(ui, self.model, caption, quantity, |value| {
                image_tools::change(self.model, self.id(), changed(value))
            });
        self.actions.extend(drafting.into_actions(self.id()));
    }

    fn position_rows(&mut self, ui: &mut Ui) {
        let keys = ["along", "across"];
        for (index, (caption, key)) in feature_fields::POSITION_CAPTIONS
            .into_iter()
            .zip(keys)
            .enumerate()
        {
            let Some(expression) = self.image.at.get(index) else {
                continue;
            };
            let image = self.image.clone();
            self.value_row(
                ui,
                caption,
                (key, expression),
                (Dimension::LENGTH, Rule::Any),
                move |value| {
                    let mut changed = image.clone();
                    if let Some(slot) = changed.at.get_mut(index) {
                        *slot = value;
                    }
                    changed
                },
            );
        }
        let image = self.image.clone();
        self.value_row(
            ui,
            ANGLE,
            ("turn", &self.image.turn),
            (Dimension::ANGLE, Rule::Any),
            move |turn| ReferenceImage {
                turn,
                ..image.clone()
            },
        );
        if let Some(flipped) = feature_fields::reverse_row(ui, FLIP, self.image.flipped) {
            self.apply(ReferenceImage {
                flipped,
                ..self.image.clone()
            });
        }
    }

    fn size_rows(&mut self, ui: &mut Ui) {
        let image = self.image.clone();
        match &self.image.size {
            ImageSize::Width(width) => self.value_row(
                ui,
                WIDTH,
                ("width", width),
                (Dimension::LENGTH, Rule::AboveZero),
                move |width| ReferenceImage {
                    size: ImageSize::Width(width),
                    ..image.clone()
                },
            ),
            ImageSize::Calibrated(calibration) => {
                let kept = calibration.clone();
                self.value_row(
                    ui,
                    DISTANCE,
                    ("distance", &calibration.distance),
                    (Dimension::LENGTH, Rule::AboveZero),
                    move |distance| ReferenceImage {
                        size: ImageSize::Calibrated(caditor_document::Calibration {
                            distance,
                            ..kept.clone()
                        }),
                        ..image.clone()
                    },
                );
            }
        }
        ui.label("");
        let mut actions = Vec::new();
        ui.vertical(|ui| {
            if feature_fields::choosing_prompt(
                ui,
                self.model,
                (self.id(), Slot::ImageCalibration),
                feature_fields::STOP_CHOOSING,
                &mut actions,
            ) {
                return;
            }
            ui.horizontal_wrapped(|ui| {
                let label = match self.image.size {
                    ImageSize::Width(_) => CALIBRATE,
                    ImageSize::Calibrated(_) => CALIBRATE_AGAIN,
                };
                let button = widgets::small_button(ui, icons::MEASURE, label);
                if ui.add(button).on_hover_text(CALIBRATE_HOVER).clicked() {
                    actions.push(Action::Editing(EditingCommand::Pick(Picking::new(
                        self.id(),
                        Slot::ImageCalibration,
                    ))));
                }
                if self.image.size.calibration().is_some() {
                    let button = widgets::small_button(ui, icons::REFERENCE_IMAGE, USE_WIDTH);
                    if ui.add(button).on_hover_text(USE_WIDTH_HOVER).clicked() {
                        let width = image_tools::placed(self.model.evaluation(), self.id())
                            .map_or(image_tools::DEFAULT_WIDTH, |placed| placed.width);
                        actions.push(feature_fields::applied(
                            &self.feature.name,
                            self.change(ReferenceImage {
                                size: ImageSize::Width(self.model.length_unit().measured(width)),
                                ..self.image.clone()
                            }),
                        ));
                    }
                }
            });
        });
        ui.end_row();
        self.actions.extend(actions);
    }

    fn display_rows(&mut self, ui: &mut Ui) {
        let current = nearest_opacity_step(self.image.opacity);
        let segments = OPACITY_STEPS
            .into_iter()
            .map(|percent| Segment {
                label: opacity_label(percent),
                hover: opacity_hover(percent),
                change: (percent != current).then(|| {
                    self.change(ReferenceImage {
                        opacity: percent,
                        ..self.image.clone()
                    })
                }),
            })
            .collect();
        let chosen = feature_fields::segmented_row(ui, "Opacity", &self.feature.name, segments);
        self.actions.extend(chosen);

        let in_front = self.image.in_front;
        let shows = [
            (false, BEHIND, BEHIND_HOVER),
            (true, IN_FRONT, IN_FRONT_HOVER),
        ]
        .into_iter()
        .map(|(front, label, hover)| Segment {
            label,
            hover,
            change: (front != in_front).then(|| {
                self.change(ReferenceImage {
                    in_front: front,
                    ..self.image.clone()
                })
            }),
        })
        .collect();
        let chosen = feature_fields::segmented_row(ui, "Shows", &self.feature.name, shows);
        self.actions.extend(chosen);

        ui.label("");
        let mut locked = self.image.locked;
        let changed = widgets::checkbox(ui, &mut locked, LOCKED)
            .on_hover_text(LOCKED_HOVER)
            .changed();
        ui.end_row();
        if changed {
            self.apply(ReferenceImage {
                locked,
                ..self.image.clone()
            });
        }
    }
}

pub fn show(
    ui: &mut Ui,
    model: &Model,
    selection: &Selection,
    actions: &mut Vec<Action>,
    feature: &Feature,
    image: &ReferenceImage,
) {
    let mut panel = Panel {
        model,
        selection,
        feature,
        image,
        actions,
    };
    widgets::properties(ui, ("image-properties", feature.id()), |ui| {
        panel.description(ui);
        panel.plane_rows(ui);
        panel.position_rows(ui);
        panel.size_rows(ui);
        panel.display_rows(ui);
    });
}
