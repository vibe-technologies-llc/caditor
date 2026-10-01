use std::path::{Path, PathBuf};

use egui::{Align, Layout, RichText, TextStyle};

use crate::{
    appearance::SPACE_M,
    dialog_parts, icons, logo,
    widgets::{self, DialogWidth},
};

pub const NAME: &str = "caditor";
pub const APP_ID: &str = "caditor";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
const PARAGRAPH_GAP: f32 = SPACE_M;
const LOGO_SIDE: f32 = 64.0;
const LICENCES_FILE: &str = "THIRD-PARTY-LICENSES.html";
const LICENCES_FOLDER: [&str; 3] = ["share", "licenses", "caditor"];
const COPY_VERSION: &str = "Copy version";
const COPY_LICENCES_PATH: &str = "Copy path";
const TAGLINE: &str = "Parametric CAD for Linux";

pub fn version_line() -> String {
    format!("{NAME} {VERSION}")
}

fn installed_licences() -> Option<PathBuf> {
    let program = std::env::current_exe().ok()?;
    licences_beside(&program)
}

fn licences_beside(program: &Path) -> Option<PathBuf> {
    let prefix = program.parent()?.parent()?;
    let path = LICENCES_FOLDER
        .iter()
        .fold(prefix.to_path_buf(), |path, part| path.join(part))
        .join(LICENCES_FILE);
    path.is_file().then_some(path)
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
        if let Some(licences) = installed_licences() {
            ui.horizontal_wrapped(|ui| {
                ui.label(widgets::muted(licences.display().to_string(), ui));
                let copy = widgets::small_button(ui, icons::COPY_PATH, COPY_LICENCES_PATH);
                if ui
                    .add(copy)
                    .on_hover_text("Copy where the licence list is, to open it in a browser")
                    .clicked()
                {
                    ui.ctx().copy_text(licences.display().to_string());
                }
            });
        }
        dialog_parts::split_footer(
            ui,
            |ui| {
                let copy = widgets::small_button(ui, icons::COPY, COPY_VERSION);
                if ui
                    .add(copy)
                    .on_hover_text("Copy the name and version, for a bug report")
                    .clicked()
                {
                    ui.ctx().copy_text(version_line());
                }
                None
            },
            |ui| {
                ui.add(widgets::primary_button(ui, "Close"))
                    .clicked()
                    .then_some(())
            },
        )
        .is_some()
    });
    response.inner || response.should_close()
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    const DESKTOP_ENTRY: &str = include_str!("../../../packaging/caditor.desktop");
    const METAINFO: &str = include_str!("../../../packaging/caditor.metainfo.xml");
    const MIME_TYPE: &str = include_str!("../../../packaging/caditor-mime.xml");
    const ARCH_PACKAGE: &str = include_str!("../../../packaging/arch/PKGBUILD");

    #[test]
    fn the_licence_list_is_found_where_the_release_installs_it() {
        let prefix = TempDir::new().unwrap();
        let program = prefix.path().join("bin").join(NAME);
        let folder = prefix.path().join("share/licenses/caditor");

        assert_eq!(licences_beside(&program), None);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join(LICENCES_FILE), "<html></html>").unwrap();
        assert_eq!(licences_beside(&program), Some(folder.join(LICENCES_FILE)));
    }

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

    #[test]
    fn the_arch_package_has_the_version_and_desktop_entry_of_this_build() {
        assert!(ARCH_PACKAGE.starts_with(&format!("pkgname={NAME}\npkgver={VERSION}\n")));
        assert!(ARCH_PACKAGE.contains(&format!("/usr/share/applications/{APP_ID}.desktop")));
        assert!(ARCH_PACKAGE.contains(&format!("/usr/bin/{NAME}\"")));
    }
}
