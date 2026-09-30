use egui::{Align, Layout, RichText, TextStyle};

use crate::{
    logo,
    widgets::{self, DialogWidth},
};

pub const NAME: &str = "caditor";
pub const APP_ID: &str = "caditor";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
const PARAGRAPH_GAP: f32 = 8.0;
const LOGO_SIDE: f32 = 64.0;
const TAGLINE: &str = "Parametric CAD for Linux";

pub fn version_line() -> String {
    format!("{NAME} {VERSION}")
}

pub fn dialog(ctx: &egui::Context) -> bool {
    let response = widgets::dialog(ctx, "about", "About caditor", DialogWidth::Medium, |ui| {
        ui.horizontal(|ui| {
            logo::show(ui, LOGO_SIDE);
            ui.with_layout(Layout::top_down(Align::Min), |ui| {
                ui.add_space(PARAGRAPH_GAP);
                ui.label(RichText::new(NAME).text_style(TextStyle::Heading));
                ui.label(widgets::muted(TAGLINE, ui));
            });
        });
        ui.add_space(PARAGRAPH_GAP);
        widgets::properties(ui, "about", |ui| {
            widgets::property(ui, "Version", |ui| ui.label(VERSION));
            widgets::property(ui, "Licence", |ui| {
                ui.label("GNU Affero General Public License, version 3 only")
            });
        });
        ui.add_space(PARAGRAPH_GAP);
        ui.label(widgets::muted(
            "caditor comes with no warranty; you may share and change it under the terms of its \
             licence.",
            ui,
        ));
        ui.add_space(PARAGRAPH_GAP);
        ui.label(widgets::muted(
            "The interface is set in Inter (SIL Open Font License 1.1) with Phosphor icons (MIT). \
             The licences of every library built into caditor are listed in \
             THIRD-PARTY-LICENSES.html, which ships with it.",
            ui,
        ));
        widgets::footer(ui, |ui| {
            ui.add(widgets::primary_button(ui, "Close")).clicked()
        })
    });
    response.inner || response.should_close()
}

#[cfg(test)]
mod tests {
    use super::*;

    const DESKTOP_ENTRY: &str = include_str!("../../../packaging/caditor.desktop");
    const METAINFO: &str = include_str!("../../../packaging/caditor.metainfo.xml");
    const MIME_TYPE: &str = include_str!("../../../packaging/caditor-mime.xml");

    #[test]
    fn the_desktop_entry_launches_and_matches_this_window() {
        assert!(DESKTOP_ENTRY.contains(&format!("\nExec={NAME} %f\n")));
        assert!(DESKTOP_ENTRY.contains(&format!("\nTryExec={NAME}\n")));
        assert!(DESKTOP_ENTRY.contains(&format!("\nStartupWMClass={APP_ID}\n")));
        assert!(DESKTOP_ENTRY.contains(&format!("\nIcon={APP_ID}\n")));
        assert!(METAINFO.contains(&format!("<id>{APP_ID}</id>")));
        assert!(METAINFO.contains(&format!(
            "<launchable type=\"desktop-id\">{APP_ID}.desktop</launchable>"
        )));
        assert!(METAINFO.contains("\n  <releases/>\n"));
    }

    #[test]
    fn the_desktop_entry_opens_the_packaged_mime_type() {
        let mime_type = "application/x-caditor";
        assert!(DESKTOP_ENTRY.contains(&format!("\nMimeType={mime_type};\n")));
        assert!(MIME_TYPE.contains(&format!("<mime-type type=\"{mime_type}\">")));
        assert!(METAINFO.contains(&format!("<mediatype>{mime_type}</mediatype>")));
    }
}
