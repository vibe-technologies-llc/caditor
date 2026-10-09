use egui::{Id, Modal, Ui};

use crate::{
    appearance::{CONTROL_HEIGHT, DIALOG_MARGIN},
    widgets::{self, Tone},
};

const MIN_BODY_HEIGHT: f32 = 96.0;

pub struct BodyRoom {
    id: Id,
    start: f32,
    pub height: f32,
}

impl BodyRoom {
    pub fn measure(ui: &Ui, id: &str, screen_share: f32) -> Self {
        let id = Id::new((id, "below-body"));
        let below = ui
            .data(|data| data.get_temp::<f32>(id))
            .unwrap_or(CONTROL_HEIGHT);
        let above = ui.cursor().min.y - ui.min_rect().min.y;
        let dialog = widgets::dialog_body(ui.ctx());
        let heading = dialog.map_or(0.0, |dialog| dialog.heading);
        let share = ui.ctx().content_rect().height() * screen_share
            - heading
            - 2.0 * f32::from(DIALOG_MARGIN);
        let room = dialog.map_or(share, |dialog| share.min(dialog.limit));
        let height = room - above - below;
        Self {
            id,
            start: 0.0,
            height: height.max(MIN_BODY_HEIGHT),
        }
    }

    pub fn body_ended(&mut self, ui: &Ui) {
        self.start = ui.cursor().min.y;
    }

    pub fn dialog_ended(&self, ui: &Ui) {
        let below = ui.cursor().min.y - self.start;
        ui.data_mut(|data| data.insert_temp(self.id, below));
    }
}

pub fn confirmation(ui: &mut Ui, question: &str, consequence: &str) {
    widgets::callout(ui, Tone::Warning, |ui| {
        ui.label(widgets::strong(question));
        ui.label(consequence);
    });
}

pub fn confirm_footer(ui: &mut Ui, confirm: &str, keep: &str) -> Option<bool> {
    widgets::footer_split(
        ui,
        |ui| {
            ui.add(widgets::danger_button(confirm))
                .clicked()
                .then_some(true)
        },
        |ui| {
            ui.add(widgets::primary_button(ui, keep))
                .clicked()
                .then_some(false)
        },
    )
}

pub fn undo_note(ui: &mut Ui, text: &str, hover: &str) -> bool {
    widgets::callout(ui, Tone::Success, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(text);
            ui.add(widgets::button("Undo"))
                .on_hover_text(hover)
                .clicked()
        })
        .inner
    })
}

pub fn titled_modal<T>(
    ctx: &egui::Context,
    id: &str,
    title: &str,
    add: impl FnOnce(&mut Ui) -> T,
) -> T {
    Modal::new(Id::new(id))
        .frame(widgets::dialog_frame(ctx))
        .show(ctx, |ui| {
            ui.set_width(widgets::fitting_width(
                ctx,
                widgets::DialogWidth::Medium.points(),
            ));
            ui.heading(title);
            ui.add_space(ui.spacing().item_spacing.y);
            add(ui)
        })
        .inner
}
