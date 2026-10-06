use std::path::PathBuf;

use caditor_document::FeatureId;
use caditor_file::{Drawing, DrawingOptions, DrawingUnit, MAX_SCALE, MIN_SCALE};
use caditor_geometry::Plane;
use egui::{Id, ScrollArea, Ui};

use crate::{
    appearance::SPACE_M,
    export, feature_tree, field,
    model::{Model, display_name},
    preferences,
    widgets::{self, DialogWidth},
};

const LAYER_LIST_HEIGHT: f32 = 140.0;
const NO_LAYERS: &str = "Choose at least one layer to import";
const NOT_A_SCALE: &str = "Enter a number from 0.000001 to 1000000";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PlaneChoice {
    #[default]
    Xy,
    Xz,
    Yz,
}

impl PlaneChoice {
    const ALL: [Self; 3] = [Self::Xy, Self::Xz, Self::Yz];

    pub fn plane(self) -> Plane {
        match self {
            Self::Xy => Plane::XY,
            Self::Xz => Plane::XZ,
            Self::Yz => Plane::YZ,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Xy => "XY",
            Self::Xz => "XZ",
            Self::Yz => "YZ",
        }
    }

    fn hover(self) -> &'static str {
        match self {
            Self::Xy => "The ground plane, seen from above",
            Self::Xz => "The front plane, seen from the front",
            Self::Yz => "The side plane, seen from the right",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Arrangement {
    pub options: DrawingOptions,
    pub plane: PlaneChoice,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ImportOptionsCommand {
    Unit(DrawingUnit),
    Scale(f64),
    Recentre(bool),
    Plane(PlaneChoice),
    Layer { layer: usize, included: bool },
    AllLayers(bool),
    Confirm,
    Cancel,
}

#[derive(Debug)]
pub struct Arranging {
    pub path: PathBuf,
    pub into: Option<FeatureId>,
    pub drawing: Drawing,
    pub arrangement: Arrangement,
}

impl Arranging {
    pub fn new(path: PathBuf, into: Option<FeatureId>, drawing: Drawing) -> Self {
        Self {
            path,
            into,
            drawing,
            arrangement: Arrangement::default(),
        }
    }

    pub fn perform(&mut self, command: ImportOptionsCommand) {
        let layers = self.drawing.layers.len();
        let Arrangement { options, plane } = &mut self.arrangement;
        match command {
            ImportOptionsCommand::Unit(unit) => options.unit = unit,
            ImportOptionsCommand::Scale(scale) => options.scale = scale,
            ImportOptionsCommand::Recentre(recentre) => options.recentre = recentre,
            ImportOptionsCommand::Plane(chosen) => *plane = chosen,
            ImportOptionsCommand::Layer {
                layer,
                included: true,
            } => {
                options.left_out_layers.remove(&layer);
            }
            ImportOptionsCommand::Layer {
                layer,
                included: false,
            } => {
                options.left_out_layers.insert(layer);
            }
            ImportOptionsCommand::AllLayers(true) => options.left_out_layers.clear(),
            ImportOptionsCommand::AllLayers(false) => {
                options.left_out_layers = (0..layers).collect()
            }
            ImportOptionsCommand::Confirm | ImportOptionsCommand::Cancel => {}
        }
    }
}

fn unit_label(unit: DrawingUnit) -> &'static str {
    match unit {
        DrawingUnit::AsInFile => "As the file says",
        DrawingUnit::Micrometres => "µm",
        DrawingUnit::Millimetres => "mm",
        DrawingUnit::Centimetres => "cm",
        DrawingUnit::Metres => "m",
    }
}

fn unit_hover(unit: DrawingUnit) -> String {
    match unit {
        DrawingUnit::AsInFile => {
            "Use the unit the drawing names, or millimetres when it names none".to_owned()
        }
        other => format!("Read the drawing's numbers as {}", other.name()),
    }
}

fn parse_scale(text: &str) -> Result<f64, String> {
    text.parse::<f64>()
        .ok()
        .filter(|scale| scale.is_finite() && (MIN_SCALE..=MAX_SCALE).contains(scale))
        .ok_or_else(|| NOT_A_SCALE.to_owned())
}

pub fn dialog(
    ctx: &egui::Context,
    model: &Model,
    arranging: &Arranging,
    into_edited_sketch: bool,
) -> Option<ImportOptionsCommand> {
    let title = format!("Import “{}”", display_name(Some(&arranging.path)));
    let response = widgets::dialog(ctx, "import-drawing", &title, DialogWidth::Medium, |ui| {
        let mut command = None;
        let Arrangement { options, plane } = &arranging.arrangement;
        let arranged = arranging.drawing.arranged(options);

        export::heading(ui, "Units");
        let units: Vec<(DrawingUnit, &str, String)> = DrawingUnit::ALL
            .iter()
            .map(|unit| (*unit, unit_label(*unit), unit_hover(*unit)))
            .collect();
        let choices: Vec<(DrawingUnit, &str, &str)> = units
            .iter()
            .map(|(unit, label, hover)| (*unit, *label, hover.as_str()))
            .collect();
        if let Some(unit) = preferences::choice(ui, &choices, options.unit) {
            command = Some(ImportOptionsCommand::Unit(unit));
        }

        export::heading(ui, "Scale");
        let field = field::commit_field(
            ui,
            Id::new("import-scale"),
            &options.scale.to_string(),
            widgets::FIELD_WIDTH,
            false,
            parse_scale,
        );
        if let Some(scale) = field.committed {
            command = Some(ImportOptionsCommand::Scale(scale));
        }
        if let Some(message) = &field.error {
            widgets::error_row(ui, message);
        }

        export::heading(ui, "Position");
        let mut recentre = options.recentre;
        if ui
            .checkbox(&mut recentre, "Centre the drawing on the origin")
            .on_hover_text("Move the middle of the drawing's outline to the sketch's origin")
            .changed()
        {
            command = Some(ImportOptionsCommand::Recentre(recentre));
        }

        if !into_edited_sketch {
            export::heading(ui, "Sketch plane");
            let planes: Vec<(PlaneChoice, &str, &str)> = PlaneChoice::ALL
                .iter()
                .map(|choice| (*choice, choice.label(), choice.hover()))
                .collect();
            if let Some(choice) = preferences::choice(ui, &planes, *plane) {
                command = Some(ImportOptionsCommand::Plane(choice));
            }
        }

        if arranging.drawing.layers.len() > 1 {
            layers(ui, arranging, &mut command);
        }

        ui.add_space(SPACE_M);
        let units = model.units();
        let summary = match arranged.bounds() {
            Some((low, high)) => format!(
                "{} drawn, {} wide and {} high.",
                feature_tree::count(arranged.curve_count(), "curve", "curves"),
                units.measured_length(high.x - low.x),
                units.measured_length(high.y - low.y),
            ),
            None => "The drawing is empty.".to_owned(),
        };
        ui.label(widgets::muted(summary, ui));

        let blocker = if !options.is_valid() {
            Some(NOT_A_SCALE)
        } else if arranged.curves.is_empty() {
            Some(NO_LAYERS)
        } else {
            None
        };
        widgets::footer(ui, |ui| {
            let import = ui.add_enabled(blocker.is_none(), widgets::primary_button(ui, "Import"));
            let import = match blocker {
                Some(reason) => import.on_disabled_hover_text(reason),
                None => import,
            };
            if import.clicked() {
                command = Some(ImportOptionsCommand::Confirm);
            }
            if ui.add(widgets::button("Cancel")).clicked() {
                command = Some(ImportOptionsCommand::Cancel);
            }
        });
        command
    });
    let closed = response
        .should_close()
        .then_some(ImportOptionsCommand::Cancel);
    response.inner.or(closed)
}

fn layers(ui: &mut Ui, arranging: &Arranging, command: &mut Option<ImportOptionsCommand>) {
    export::heading(ui, "Layers");
    let left_out = &arranging.arrangement.options.left_out_layers;
    ui.horizontal(|ui| {
        if ui.add(widgets::button("Select all")).clicked() {
            *command = Some(ImportOptionsCommand::AllLayers(true));
        }
        if ui.add(widgets::button("Select none")).clicked() {
            *command = Some(ImportOptionsCommand::AllLayers(false));
        }
    });
    let height = widgets::list_height(ui.ctx(), LAYER_LIST_HEIGHT);
    widgets::card(ui, |ui| {
        ScrollArea::vertical().max_height(height).show(ui, |ui| {
            for (layer, name) in arranging.drawing.layers.iter().enumerate() {
                let curves = arranging.drawing.layer_curve_count(layer);
                let label = format!(
                    "{name} ({})",
                    feature_tree::count(curves, "curve", "curves")
                );
                let mut included = !left_out.contains(&layer);
                if ui.checkbox(&mut included, label).changed() {
                    *command = Some(ImportOptionsCommand::Layer { layer, included });
                }
            }
        });
    });
}
