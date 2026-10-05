use caditor_document::{
    BodyAppearance, Document, Edit, Feature, FeatureId, MAX_MATERIAL_NAME_CHARS, Rgb, Transaction,
    density_of, material_name,
};
use caditor_expression::{Dimension, Expression};
use egui::{Color32, Id, Ui};

use crate::{
    appearance::SPACE_S,
    feature_fields::{self, Choice},
    field::{self, Expected},
    model::{Action, Model},
    widgets::{self, FIELD_WIDTH},
};

pub const DEFAULT_COLOUR: Rgb = Rgb::new(150, 162, 180);
pub const DEFAULT_COLOUR_NAME: &str = "Default colour";
pub const DENSITY_UNIT: &str = "g/cm³";
pub const NO_MATERIAL: &str = "None";
pub const COLOUR_CAPTION: &str = "Colour";
pub const MATERIAL_CAPTION: &str = "Material";
pub const MATERIAL_NAME_CAPTION: &str = "Name";
pub const DENSITY_CAPTION: &str = "Density";
const HEX_HINT: &str = "Enter a colour as # and six hexadecimal digits, such as #4682b4";
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

    fn swatches(&mut self, ui: &mut Ui, focus: bool) -> bool {
        let current = self.appearance.colour;
        let (chosen, focused) = ui
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
                (chosen, default.has_focus())
            })
            .inner;
        if let Some(colour) = chosen.filter(|colour| *colour != current) {
            self.apply(with_colour(self.appearance, colour), "colour");
        }
        focused
    }

    fn colour_row(&mut self, ui: &mut Ui) {
        widgets::caption(ui, COLOUR_CAPTION);
        let current = self.appearance.colour;
        let stored = current.map(Rgb::hex).unwrap_or_default();
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
                    .ok_or_else(|| HEX_HINT.to_owned())
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
    actions: &mut Vec<Action>,
    feature: &Feature,
    focus: bool,
) -> bool {
    let mut panel = Panel {
        model,
        body: feature.id(),
        appearance: &feature.appearance,
        actions,
    };
    let focused = panel.swatches(ui, focus);
    ui.add_space(SPACE_S);
    widgets::properties(ui, ("body-appearance", feature.id()), |ui| {
        panel.colour_row(ui);
        panel.material_rows(ui);
        panel.density_row(ui);
    });
    focused
}
