use caditor_document::{Document, Transaction};
use egui::{Align, Layout, ScrollArea};

use crate::{
    appearance::SPACE_M,
    feature_tree::count,
    model::Model,
    widgets::{self, DialogWidth},
};

pub const TITLE: &str = "Undo history";
pub const NOTHING_YET: &str = "Nothing has been changed yet.";
pub const NOW: &str = "Now";
pub const UNDONE: &str = "Undone, Redo brings them back";
const LIST_HEIGHT: f32 = 360.0;
const NAMED: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Jump {
    Close,
    Undo(usize),
    Redo(usize),
}

pub fn dialog(ctx: &egui::Context, model: &Model) -> Option<Jump> {
    let response = widgets::dialog(ctx, "undo-history", TITLE, DialogWidth::Medium, |ui| {
        ui.label(widgets::muted(
            "Choose a change to go to the model as it was just after it.",
            ui,
        ));
        ui.add_space(SPACE_M);
        let document = model.document();
        let undone: Vec<&Transaction> = model.redo_steps().collect();
        let done: Vec<&Transaction> = model.undo_steps().collect();
        let mut jump = None;
        ScrollArea::vertical()
            .max_height(LIST_HEIGHT)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                if !undone.is_empty() {
                    ui.label(widgets::muted(UNDONE, ui));
                }
                for (index, step) in undone.iter().enumerate().rev() {
                    if step_button(ui, document, step).clicked() {
                        jump = Some(Jump::Redo(index + 1));
                    }
                }
                ui.separator();
                ui.label(NOW);
                if done.is_empty() && undone.is_empty() {
                    ui.label(widgets::muted(NOTHING_YET, ui));
                }
                for (index, step) in done.iter().enumerate() {
                    if step_button(ui, document, step).clicked() && index > 0 {
                        jump = Some(Jump::Undo(index));
                    }
                }
            });
        ui.add_space(SPACE_M);
        let closed = ui
            .with_layout(Layout::top_down(Align::Min), |ui| {
                widgets::footer_split(
                    ui,
                    |_| None,
                    |ui| {
                        ui.add(widgets::primary_button(ui, "Close"))
                            .clicked()
                            .then_some(())
                    },
                )
            })
            .inner
            .is_some();
        jump.or(closed.then_some(Jump::Close))
    });
    response
        .inner
        .or(response.should_close().then_some(Jump::Close))
}

fn step_button(ui: &mut egui::Ui, document: &Document, step: &Transaction) -> egui::Response {
    let response = ui.add(widgets::button(step.label()));
    let summary = summary(document, step);
    if summary.is_empty() {
        response
    } else {
        response.on_hover_text(summary)
    }
}

pub fn summary(document: &Document, step: &Transaction) -> String {
    let touched = step.touched();
    let mut lines = Vec::new();
    let features: Vec<String> = touched
        .features
        .iter()
        .map(|id| {
            document
                .feature(*id)
                .map(|feature| feature.name.clone())
                .or_else(|| touched.named_features.get(id).cloned())
                .unwrap_or_else(|| "a deleted feature".to_owned())
        })
        .collect();
    if !features.is_empty() {
        lines.push(format!("Features: {}", named_list(&features)));
    }
    let mut sketch = Vec::new();
    if !touched.entities.is_empty() {
        sketch.push(count(
            touched.entities.len(),
            "curve or point",
            "curves and points",
        ));
    }
    if !touched.constraints.is_empty() {
        sketch.push(count(
            touched.constraints.len(),
            "constraint or dimension",
            "constraints and dimensions",
        ));
    }
    if !sketch.is_empty() {
        lines.push(format!("Sketch: {}", sketch.join(", ")));
    }
    let parameters: Vec<String> = touched
        .parameters
        .iter()
        .map(|id| {
            document
                .parameter(*id)
                .map(|parameter| parameter.name.clone())
                .or_else(|| touched.named_parameters.get(id).cloned())
                .unwrap_or_else(|| "a deleted parameter".to_owned())
        })
        .collect();
    if !parameters.is_empty() {
        lines.push(format!("Parameters: {}", named_list(&parameters)));
    }
    if touched.rollback {
        lines.push("The rollback bar".to_owned());
    }
    if touched.principal {
        lines.push("The principal planes, axes or origin".to_owned());
    }
    lines.join("\n")
}

fn named_list(names: &[String]) -> String {
    let shown: Vec<&str> = names.iter().take(NAMED).map(String::as_str).collect();
    match names.len().saturating_sub(NAMED) {
        0 => shown.join(", "),
        more => format!("{} and {more} more", shown.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use caditor_document::{Edit, FeatureKind};
    use caditor_geometry::{Plane, Point2};
    use caditor_sketch::Sketch;

    use super::*;

    #[test]
    fn a_step_is_summed_up_by_the_features_sketch_items_and_parameters_it_touched() {
        let mut document = Document::default();
        let mut sketch = Sketch::new(Plane::XY);
        sketch.add_line(Point2::ZERO, Point2::new(10.0, 0.0));
        let mut transaction = document.transaction("Build");
        let base = transaction.add_feature("Base", FeatureKind::from(sketch));
        transaction.add_parameter("width", transaction.parse("5 mm").unwrap());
        let built = transaction.finish();
        document.apply(built.clone()).unwrap();
        let hidden = Transaction::single(
            "Hide",
            Edit::SetFeatureHidden {
                id: base,
                hidden: true,
            },
        );

        assert_eq!(
            summary(&document, &built),
            "Features: Base\nParameters: width"
        );
        assert_eq!(summary(&document, &hidden), "Features: Base");
        assert_eq!(
            named_list(&(1..=6).map(|index| index.to_string()).collect::<Vec<_>>()),
            "1, 2, 3, 4 and 2 more"
        );
    }
}
