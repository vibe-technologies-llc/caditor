use caditor_document::{
    BodyAppearance, Document, Edit, FaceColour, Feature, FeatureId, MAX_BODY_NAME_CHARS,
    MAX_MATERIAL_NAME_CHARS, Rgb, Transaction, density_of, material_name,
};
use caditor_expression::{Dimension, Expression};
use caditor_kernel::FaceReference;
use egui::{Color32, Id, Ui, Widget};

use crate::{
    appearance::SPACE_S,
    bodies, colour_selector,
    feature_fields::{self, Choice},
    field::{self, Expected},
    icons,
    model::{Action, Model},
    preferences::{PreferenceChange, PreferencesCommand},
    selection::{Pickable, Selection},
    widgets::{self, FIELD_WIDTH},
};

pub const DEFAULT_COLOUR: Rgb = caditor_document::DEFAULT_BODY_COLOUR;
pub const DEFAULT_COLOUR_NAME: &str = "Default colour";
pub const BODY_COLOUR_NAME: &str = "The body's colour";
pub const CUSTOM_COLOUR_NAME: &str = "Custom colour";
pub const CLEAR_FACE_COLOURS: &str = "Clear face colours";
pub const DENSITY_UNIT: &str = "g/cm³";
pub const NO_MATERIAL: &str = "None";
pub const COLOUR_CAPTION: &str = "Colour";
pub const MATERIAL_CAPTION: &str = "Material";
pub const MATERIAL_NAME_CAPTION: &str = "Name";
pub const DENSITY_CAPTION: &str = "Density";
pub const BODY_NAME_CAPTION: &str = "Body name";
pub const OPACITY_CAPTION: &str = "Opacity";
pub const OPACITIES: [(Option<u8>, &str, &str); 4] = [
    (None, "Solid", "Draw the body solid"),
    (
        Some(75),
        "75%",
        "Let a little of what is behind the body show through; its faces are not picked",
    ),
    (
        Some(50),
        "50%",
        "Draw the body half see-through; its faces are not picked",
    ),
    (
        Some(25),
        "25%",
        "Draw the body faint, mostly see-through; its faces are not picked",
    ),
];
const DENSITY_NOTE: &str =
    "A plain number in g/cm³; the Measure panel shows the body's mass from it.";

pub struct Swatch {
    pub name: &'static str,
    pub colour: Rgb,
}

pub const SWATCHES: [Swatch; 11] = [
    Swatch {
        name: "Steel blue",
        colour: Rgb::new(70, 130, 180),
    },
    Swatch {
        name: "Blue",
        colour: Rgb::new(52, 101, 196),
    },
    Swatch {
        name: "Teal",
        colour: Rgb::new(38, 150, 150),
    },
    Swatch {
        name: "Green",
        colour: Rgb::new(76, 160, 90),
    },
    Swatch {
        name: "Yellow",
        colour: Rgb::new(228, 192, 52),
    },
    Swatch {
        name: "Orange",
        colour: Rgb::new(230, 126, 34),
    },
    Swatch {
        name: "Red",
        colour: Rgb::new(200, 64, 52),
    },
    Swatch {
        name: "Purple",
        colour: Rgb::new(130, 90, 180),
    },
    Swatch {
        name: "Brown",
        colour: Rgb::new(150, 100, 60),
    },
    Swatch {
        name: "Charcoal",
        colour: Rgb::new(60, 62, 68),
    },
    Swatch {
        name: "White",
        colour: Rgb::new(236, 236, 236),
    },
];

pub struct Material {
    pub name: &'static str,
    pub density: f64,
    pub colour: Option<Rgb>,
}

pub const MATERIALS: [Material; 12] = [
    Material {
        name: "Aluminium",
        density: 2.70,
        colour: Some(Rgb::new(190, 196, 204)),
    },
    Material {
        name: "Steel",
        density: 7.85,
        colour: Some(Rgb::new(138, 144, 153)),
    },
    Material {
        name: "Stainless steel",
        density: 8.00,
        colour: Some(Rgb::new(170, 175, 181)),
    },
    Material {
        name: "Brass",
        density: 8.50,
        colour: Some(Rgb::new(201, 164, 64)),
    },
    Material {
        name: "Copper",
        density: 8.96,
        colour: Some(Rgb::new(184, 115, 51)),
    },
    Material {
        name: "Titanium",
        density: 4.51,
        colour: Some(Rgb::new(135, 134, 129)),
    },
    Material {
        name: "ABS",
        density: 1.04,
        colour: None,
    },
    Material {
        name: "PLA",
        density: 1.24,
        colour: None,
    },
    Material {
        name: "PETG",
        density: 1.27,
        colour: None,
    },
    Material {
        name: "Nylon (PA12)",
        density: 1.01,
        colour: None,
    },
    Material {
        name: "Polycarbonate",
        density: 1.20,
        colour: None,
    },
    Material {
        name: "Oak",
        density: 0.75,
        colour: Some(Rgb::new(160, 120, 74)),
    },
];

pub fn color32(colour: Rgb) -> Color32 {
    Color32::from_rgb(colour.red, colour.green, colour.blue)
}

pub fn change(
    document: &Document,
    body: FeatureId,
    appearance: BodyAppearance,
    what: &str,
) -> Result<Transaction, String> {
    let feature = document
        .feature(body)
        .ok_or_else(|| "The body no longer exists".to_owned())?;
    let label = format!("Change the {what} of {}", feature.name);
    field::checked(
        document,
        Transaction::single(
            label,
            Edit::SetBodyAppearance {
                id: body,
                appearance,
            },
        ),
    )
}

pub fn with_colour(appearance: &BodyAppearance, colour: Option<Rgb>) -> BodyAppearance {
    BodyAppearance {
        colour,
        ..appearance.clone()
    }
}

pub fn with_face_colours(
    appearance: &BodyAppearance,
    faces: &[FaceReference],
    colour: Option<Rgb>,
) -> BodyAppearance {
    let mut faces_coloured: Vec<FaceColour> = appearance
        .faces
        .iter()
        .filter(|coloured| faces.iter().all(|face| face.name() != coloured.face.name()))
        .cloned()
        .collect();
    if let Some(colour) = colour {
        let opacity_of = |face: &FaceReference| {
            appearance
                .faces
                .iter()
                .rev()
                .find(|coloured| coloured.face.name() == face.name())
                .and_then(|coloured| coloured.opacity)
        };
        faces_coloured.extend(faces.iter().map(|face| FaceColour {
            face: face.clone(),
            colour,
            opacity: opacity_of(face),
        }));
    }
    BodyAppearance {
        faces: faces_coloured,
        ..appearance.clone()
    }
}

pub fn faces_of_other_bodies(selection: &Selection, body: FeatureId) -> usize {
    selection
        .iter()
        .filter(|pickable| matches!(pickable, Pickable::Face { body: owner, .. } if *owner != body))
        .count()
}

pub fn other_faces_note(count: usize) -> Option<String> {
    match count {
        0 => None,
        1 => Some(
            "The selected face of another body is left out; colour it from its own body".to_owned(),
        ),
        count => Some(format!(
            "The {count} selected faces of other bodies are left out; colour them from their own \
             bodies"
        )),
    }
}

pub fn selector_id(body: FeatureId, faces: bool) -> Id {
    Id::new(("custom-colour", body, faces))
}

pub fn recent_swatch_name(colour: Rgb) -> String {
    format!("Recent colour, {}", colour_selector::describe(colour))
}

pub fn face_swatch_name(colour: &str) -> String {
    format!("{colour} for the selected faces")
}

pub fn selected_faces(model: &Model, selection: &Selection, body: FeatureId) -> Vec<FaceReference> {
    let Some(shown) = bodies::shown(model.evaluation(), body) else {
        return Vec::new();
    };
    selection
        .iter()
        .filter_map(|pickable| match pickable {
            Pickable::Face { body: owner, face } if owner == body => {
                FaceReference::capture(&shown.solid, bodies::find_face(shown, face)?)
            }
            _ => None,
        })
        .collect()
}

pub fn with_material(appearance: &BodyAppearance, material: Option<&Material>) -> BodyAppearance {
    match material {
        None => BodyAppearance {
            material: None,
            density: None,
            ..appearance.clone()
        },
        Some(material) => BodyAppearance {
            colour: appearance.colour.or(material.colour),
            material: Some(material.name.to_owned()),
            density: Some(Expression::number(material.density)),
            name: appearance.name.clone(),
            opacity: appearance.opacity,
            faces: appearance.faces.clone(),
        },
    }
}

pub fn parse_density(model: &Model, text: &str) -> Result<Option<Expression>, String> {
    if text.trim().is_empty() {
        return Ok(None);
    }
    let parameters = model.parameters();
    let expression = field::parse_expression(
        model.document(),
        parameters,
        text,
        Expected {
            dimension: Some(Dimension::NONE),
            non_negative: false,
        },
        model.units(),
    )?;
    density_of(&expression, parameters).map_err(|error| field::sentence(&error.to_string()))?;
    Ok(Some(expression))
}

struct Panel<'a> {
    model: &'a Model,
    body: FeatureId,
    appearance: &'a BodyAppearance,
    actions: &'a mut Vec<Action>,
}

impl Panel<'_> {
    fn apply(&mut self, appearance: BodyAppearance, what: &str) {
        let change = change(self.model.document(), self.body, appearance, what);
        let name = feature_fields::feature_name(self.model.document(), self.body).unwrap_or("");
        self.actions.push(feature_fields::applied(name, change));
    }

    fn remember(&mut self, colour: Rgb) {
        self.actions
            .push(Action::Preferences(PreferencesCommand::Change(
                PreferenceChange::RecentColour(colour),
            )));
    }

    fn swatches(&mut self, ui: &mut Ui, focus: bool) -> bool {
        let current = self.appearance.colour;
        let selector = selector_id(self.body, false);
        let recent = self.model.recent_colours();
        let selecting = colour_selector::is_open(ui, selector);
        let (chosen, focused, toggled) = ui
            .horizontal_wrapped(|ui| {
                let mut chosen = None;
                let default = widgets::swatch(
                    ui,
                    color32(DEFAULT_COLOUR),
                    DEFAULT_COLOUR_NAME,
                    current.is_none(),
                );
                if focus {
                    default.request_focus();
                    default.scroll_to_me(Some(egui::Align::Center));
                }
                if default.clicked() {
                    chosen = Some(None);
                }
                for swatch in &SWATCHES {
                    let picked = current == Some(swatch.colour);
                    if widgets::swatch(ui, color32(swatch.colour), swatch.name, picked).clicked() {
                        chosen = Some(Some(swatch.colour));
                    }
                }
                for colour in recent {
                    let picked = current == Some(*colour);
                    let name = recent_swatch_name(*colour);
                    if widgets::swatch(ui, color32(*colour), &name, picked).clicked() {
                        chosen = Some(Some(*colour));
                    }
                }
                let toggled =
                    widgets::icon_swatch(ui, icons::ADD, CUSTOM_COLOUR_NAME, selecting).clicked();
                (chosen, default.has_focus(), toggled)
            })
            .inner;
        let shown = current.unwrap_or(DEFAULT_COLOUR);
        if toggled {
            colour_selector::toggle(ui, selector, shown);
        }
        if let Some(colour) = chosen.filter(|colour| *colour != current) {
            if let Some(colour) = colour {
                self.remember(colour);
            }
            self.apply(with_colour(self.appearance, colour), "colour");
        }
        if let Some(colour) = colour_selector::show(ui, selector, shown) {
            self.remember(colour);
            self.apply(with_colour(self.appearance, Some(colour)), "colour");
        }
        focused
    }

    fn face_swatches(&mut self, ui: &mut Ui, faces: &[FaceReference]) {
        if !faces.is_empty() {
            let caption = match faces.len() {
                1 => "Colour the selected face".to_owned(),
                count => format!("Colour the {count} selected faces"),
            };
            ui.label(widgets::muted(caption, ui));
            let current: Vec<Option<Rgb>> = faces
                .iter()
                .map(|face| {
                    self.appearance
                        .faces
                        .iter()
                        .rev()
                        .find(|coloured| coloured.face.name() == face.name())
                        .map(|coloured| coloured.colour)
                })
                .collect();
            let shared = current
                .first()
                .copied()
                .filter(|first| current.iter().all(|colour| colour == first));
            let selector = selector_id(self.body, true);
            let recent = self.model.recent_colours();
            let selecting = colour_selector::is_open(ui, selector);
            let (chosen, toggled) = ui
                .horizontal_wrapped(|ui| {
                    let mut chosen = None;
                    let default = widgets::swatch(
                        ui,
                        color32(self.appearance.colour.unwrap_or(DEFAULT_COLOUR)),
                        &face_swatch_name(BODY_COLOUR_NAME),
                        shared == Some(None),
                    );
                    if default.clicked() {
                        chosen = Some(None);
                    }
                    for swatch in &SWATCHES {
                        let picked = shared == Some(Some(swatch.colour));
                        let name = face_swatch_name(swatch.name);
                        if widgets::swatch(ui, color32(swatch.colour), &name, picked).clicked() {
                            chosen = Some(Some(swatch.colour));
                        }
                    }
                    for colour in recent {
                        let picked = shared == Some(Some(*colour));
                        let name = face_swatch_name(&recent_swatch_name(*colour));
                        if widgets::swatch(ui, color32(*colour), &name, picked).clicked() {
                            chosen = Some(Some(*colour));
                        }
                    }
                    let name = face_swatch_name(CUSTOM_COLOUR_NAME);
                    let toggled = widgets::icon_swatch(ui, icons::ADD, &name, selecting).clicked();
                    (chosen, toggled)
                })
                .inner;
            let shown = shared
                .flatten()
                .or(self.appearance.colour)
                .unwrap_or(DEFAULT_COLOUR);
            if toggled {
                colour_selector::toggle(ui, selector, shown);
            }
            if let Some(colour) = chosen {
                if let Some(colour) = colour {
                    self.remember(colour);
                }
                self.apply(
                    with_face_colours(self.appearance, faces, colour),
                    "face colours",
                );
            }
            if let Some(colour) = colour_selector::show(ui, selector, shown) {
                self.remember(colour);
                self.apply(
                    with_face_colours(self.appearance, faces, Some(colour)),
                    "face colours",
                );
            }
        }
        if !self.appearance.faces.is_empty()
            && widgets::small_button(ui, icons::REMOVE, CLEAR_FACE_COLOURS)
                .ui(ui)
                .clicked()
        {
            let cleared = BodyAppearance {
                faces: Vec::new(),
                ..self.appearance.clone()
            };
            self.apply(cleared, "face colours");
        }
    }

    fn name_row(&mut self, ui: &mut Ui, feature: &Feature, focus: bool) -> bool {
        widgets::caption(ui, BODY_NAME_CAPTION);
        let stored = self.appearance.name.clone().unwrap_or_default();
        let field = field::commit_field(
            ui,
            name_field_id(self.body),
            &stored,
            FIELD_WIDTH,
            focus,
            |text| {
                let length = text.trim().chars().count();
                if length > MAX_BODY_NAME_CHARS {
                    return Err(format!(
                        "Enter a name of at most {MAX_BODY_NAME_CHARS} characters; this one has \
                         {length}"
                    ));
                }
                Ok(material_name(text))
            },
        );
        let landed = field.response.has_focus();
        field.response.on_hover_text(format!(
            "The body's own name, used in the Bodies list and in exports; empty, it is named \
             after {}",
            feature.name
        ));
        ui.end_row();
        if let Some(error) = field.error {
            widgets::error_row(ui, &error);
        }
        if let Some(name) = field.committed {
            let named = BodyAppearance {
                name,
                ..self.appearance.clone()
            };
            self.apply(named, "name");
        }
        landed
    }

    fn colour_row(&mut self, ui: &mut Ui) {
        widgets::caption(ui, COLOUR_CAPTION);
        let current = self.appearance.colour;
        let working = colour_selector::working(ui, selector_id(self.body, false));
        let stored = working.or(current).map(Rgb::hex).unwrap_or_default();
        let field = field::commit_field(
            ui,
            Id::new(("body-colour", self.body)),
            &stored,
            FIELD_WIDTH,
            false,
            |text| {
                if text.is_empty() {
                    return Ok(None);
                }
                Rgb::from_hex(text)
                    .map(Some)
                    .ok_or_else(|| colour_selector::HEX_HINT.to_owned())
            },
        );
        ui.end_row();
        if let Some(error) = field.error {
            widgets::error_row(ui, &error);
        }
        if let Some(colour) = field.committed {
            self.apply(with_colour(self.appearance, colour), "colour");
        }
    }

    fn opacity_row(&mut self, ui: &mut Ui) {
        widgets::caption(ui, OPACITY_CAPTION);
        let current = self.appearance.opacity;
        let shown = OPACITIES
            .iter()
            .find(|(opacity, _, _)| *opacity == current)
            .map_or_else(
                || format!("{}%", current.unwrap_or(100)),
                |(_, label, _)| (*label).to_owned(),
            );
        let model = self.model;
        let body = self.body;
        let appearance = self.appearance;
        let chosen = feature_fields::combo(ui, Id::new(("body-opacity", body)), shown, || {
            OPACITIES
                .iter()
                .map(|(opacity, label, _)| Choice {
                    label: (*label).to_owned(),
                    selected: *opacity == current,
                    change: change(
                        model.document(),
                        body,
                        BodyAppearance {
                            opacity: *opacity,
                            ..appearance.clone()
                        },
                        "opacity",
                    )
                    .map(Action::Apply),
                })
                .collect()
        });
        ui.end_row();
        self.actions.extend(chosen);
    }

    fn material_rows(&mut self, ui: &mut Ui) {
        widgets::caption(ui, MATERIAL_CAPTION);
        let current = self.appearance.material.as_deref();
        let shown = current.unwrap_or(NO_MATERIAL).to_owned();
        let model = self.model;
        let body = self.body;
        let appearance = self.appearance;
        let chosen = feature_fields::combo(ui, Id::new(("body-material", body)), shown, || {
            std::iter::once((NO_MATERIAL, None))
                .chain(
                    MATERIALS
                        .iter()
                        .map(|material| (material.name, Some(material))),
                )
                .map(|(label, material)| {
                    let selected = match material {
                        None => current.is_none(),
                        Some(material) => Some(material.name) == current,
                    };
                    let change = change(
                        model.document(),
                        body,
                        with_material(appearance, material),
                        "material",
                    )
                    .map(Action::Apply);
                    Choice {
                        label: label.to_owned(),
                        selected,
                        change,
                    }
                })
                .collect()
        });
        ui.end_row();
        self.actions.extend(chosen);

        widgets::caption(ui, MATERIAL_NAME_CAPTION);
        let field = field::commit_field(
            ui,
            Id::new(("body-material-name", self.body)),
            current.unwrap_or_default(),
            FIELD_WIDTH,
            false,
            |text| {
                let length = text.chars().count();
                if length > MAX_MATERIAL_NAME_CHARS {
                    return Err(format!(
                        "Enter a name of at most {MAX_MATERIAL_NAME_CHARS} characters; this one \
                         has {length}"
                    ));
                }
                Ok(material_name(text))
            },
        );
        ui.end_row();
        if let Some(error) = field.error {
            widgets::error_row(ui, &error);
        }
        if let Some(material) = field.committed {
            let named = BodyAppearance {
                material,
                ..self.appearance.clone()
            };
            self.apply(named, "material");
        }
    }

    fn density_row(&mut self, ui: &mut Ui) {
        widgets::caption(ui, DENSITY_CAPTION);
        let document = self.model.document();
        let stored = self
            .appearance
            .density
            .as_ref()
            .map(|density| document.expression_text(density))
            .unwrap_or_default();
        let model = self.model;
        let field = ui
            .horizontal(|ui| {
                let field = field::commit_field(
                    ui,
                    Id::new(("body-density", self.body)),
                    &stored,
                    FIELD_WIDTH,
                    false,
                    |text| parse_density(model, text),
                );
                ui.label(widgets::muted(DENSITY_UNIT, ui));
                let preview = self
                    .appearance
                    .density
                    .as_ref()
                    .filter(|_| field.error.is_none())
                    .and_then(|density| {
                        field::value_preview(model.parameters(), density, model.units())
                    });
                if let Some(preview) = preview {
                    ui.label(widgets::muted(preview, ui));
                }
                field
            })
            .inner;
        ui.end_row();
        if let Some(error) = field.error {
            widgets::error_row(ui, &error);
        } else if let Some(Err(error)) = self.appearance.density_value(model.parameters()) {
            widgets::error_row(ui, &field::sentence(&error.to_string()));
        }
        if let Some(density) = field.committed {
            let weighed = BodyAppearance {
                density,
                ..self.appearance.clone()
            };
            self.apply(weighed, "density");
        }
        feature_fields::description_row(ui, DENSITY_NOTE);
    }
}

pub fn show(
    ui: &mut Ui,
    model: &Model,
    selection: &Selection,
    actions: &mut Vec<Action>,
    feature: &Feature,
    (focus, naming): (bool, bool),
) -> bool {
    let mut panel = Panel {
        model,
        body: feature.id(),
        appearance: &feature.appearance,
        actions,
    };
    let swatch_focused = panel.swatches(ui, focus && !naming);
    ui.add_space(SPACE_S);
    let faces = selected_faces(model, selection, feature.id());
    if !faces.is_empty() || !feature.appearance.faces.is_empty() {
        panel.face_swatches(ui, &faces);
        ui.add_space(SPACE_S);
    }
    if let Some(note) = other_faces_note(faces_of_other_bodies(selection, feature.id())) {
        ui.label(widgets::muted(note, ui));
        ui.add_space(SPACE_S);
    }
    let name_focused = widgets::properties(ui, ("body-appearance", feature.id()), |ui| {
        let landed = panel.name_row(ui, feature, focus && naming);
        panel.colour_row(ui);
        panel.opacity_row(ui);
        panel.material_rows(ui);
        panel.density_row(ui);
        landed
    });
    if naming { name_focused } else { swatch_focused }
}

pub fn name_field_id(body: FeatureId) -> Id {
    Id::new(("body-name", body))
}

#[cfg(test)]
mod tests {
    use caditor_document::OPACITY_STEPS;

    use super::OPACITIES;

    #[test]
    fn the_opacity_choices_are_solid_and_the_steps_an_import_snaps_to() {
        let offered: Vec<Option<u8>> = OPACITIES.iter().map(|(opacity, _, _)| *opacity).collect();
        let expected: Vec<Option<u8>> = std::iter::once(None)
            .chain(OPACITY_STEPS.into_iter().map(Some))
            .collect();

        assert_eq!(offered, expected);
    }
}
