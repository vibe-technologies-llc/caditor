use caditor_document::{Document, Feature, FeatureKind};

pub fn show(ui: &mut egui::Ui, document: &Document) {
    egui::Panel::left("model")
        .resizable(true)
        .default_size(240.0)
        .show(ui, |ui| {
            ui.heading("Features");
            for feature in document.features() {
                ui.label(feature_summary(feature));
            }

            ui.separator();
            ui.heading("Parameters");
            egui::Grid::new("parameters").striped(true).show(ui, |ui| {
                for parameter in document.parameters() {
                    ui.label(&parameter.name);
                    ui.label(format!("{} mm", parameter.value));
                    ui.end_row();
                }
            });
        });
}

fn feature_summary(feature: &Feature) -> String {
    match &feature.kind {
        FeatureKind::Sketch(sketch) => format!(
            "{} ({}, {})",
            feature.name,
            count(sketch.entities().len(), "entity", "entities"),
            count(sketch.constraints().len(), "constraint", "constraints")
        ),
    }
}

fn count(amount: usize, singular: &str, plural: &str) -> String {
    let noun = if amount == 1 { singular } else { plural };
    format!("{amount} {noun}")
}
