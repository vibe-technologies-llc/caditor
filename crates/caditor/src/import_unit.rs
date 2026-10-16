use std::path::PathBuf;

use caditor_file::{MeshFormat, MeshUnit, ModelImport, UnitReading};
use egui::Label;

use crate::{
    appearance::SPACE_M,
    export,
    model::{Model, display_name},
    preferences,
    widgets::{self, DialogWidth, Tone},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportUnitCommand {
    Choose(MeshUnit),
    Confirm,
    Cancel,
}

#[derive(Debug)]
pub struct Scaling {
    pub path: PathBuf,
    pub session: u64,
    pub imported: ModelImport,
    pub reading: UnitReading,
    pub chosen: MeshUnit,
}

impl Scaling {
    pub fn new(path: PathBuf, session: u64, imported: ModelImport, reading: UnitReading) -> Self {
        Self {
            path,
            session,
            imported,
            reading,
            chosen: reading.unit,
        }
    }

    pub fn keeps_the_guess(&self) -> bool {
        self.chosen == self.reading.unit
    }

    fn format_name(&self) -> &'static str {
        MeshFormat::of(&self.path).map_or("mesh", MeshFormat::name)
    }
}

fn unit_label(unit: MeshUnit) -> &'static str {
    match unit {
        MeshUnit::Micrometres => "µm",
        MeshUnit::Millimetres => "mm",
        MeshUnit::Centimetres => "cm",
        MeshUnit::Metres => "m",
    }
}

fn unit_hover(unit: MeshUnit) -> String {
    format!("Read the file's numbers as {}", unit.name())
}

pub fn explanation(scaling: &Scaling, model: &Model) -> String {
    let units = model.units();
    let as_written = scaling
        .reading
        .size_in_millimetres(MeshUnit::Millimetres)
        .max_element();
    format!(
        "{} files carry no units. Read as millimetres, the longest side of this part would be \
         only {}, which is unlikely for a part, so caditor guessed {}. Choose the unit the part \
         was made in.",
        scaling.format_name(),
        units.measured_length(as_written),
        scaling.reading.unit.name(),
    )
}

pub fn size_summary(scaling: &Scaling, model: &Model) -> String {
    let units = model.units();
    let size = scaling.reading.size_in_millimetres(scaling.chosen);
    format!(
        "Read as {}, the part is {} by {} by {}.",
        scaling.chosen.name(),
        units.measured_length(size.x),
        units.measured_length(size.y),
        units.measured_length(size.z),
    )
}

pub fn dialog(ctx: &egui::Context, model: &Model, scaling: &Scaling) -> Option<ImportUnitCommand> {
    let title = format!("Import “{}”", display_name(Some(&scaling.path)));
    let response = widgets::dialog(ctx, "import-unit", &title, DialogWidth::Medium, |ui| {
        let mut command = None;
        widgets::dialog_body(ui, |ui| {
            widgets::callout(ui, Tone::Info, |ui| {
                ui.add(Label::new(explanation(scaling, model)).wrap());
            });

            export::heading(ui, "Units");
            let choices: Vec<(MeshUnit, &str, String)> = MeshUnit::ALL
                .iter()
                .map(|unit| (*unit, unit_label(*unit), unit_hover(*unit)))
                .collect();
            let options: Vec<(MeshUnit, &str, &str)> = choices
                .iter()
                .map(|(unit, label, hover)| (*unit, *label, hover.as_str()))
                .collect();
            if let Some(unit) = preferences::choice(ui, &options, scaling.chosen) {
                command = Some(ImportUnitCommand::Choose(unit));
            }

            ui.add_space(SPACE_M);
            ui.label(widgets::muted(size_summary(scaling, model), ui));
        });
        widgets::footer(ui, |ui| {
            if ui.add(widgets::primary_button(ui, "Import")).clicked() {
                command = Some(ImportUnitCommand::Confirm);
            }
            if ui.add(widgets::button("Cancel")).clicked() {
                command = Some(ImportUnitCommand::Cancel);
            }
        });
        command
    });
    let closed = response.should_close().then_some(ImportUnitCommand::Cancel);
    response.inner.or(closed)
}
