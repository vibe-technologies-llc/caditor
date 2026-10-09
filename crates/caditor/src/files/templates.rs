use std::{
    ffi::OsStr,
    fs, io,
    path::{Path, PathBuf},
    sync::Arc,
};

use caditor_document::Document;
use caditor_file::{
    FILE_EXTENSION, LoadError, Loaded, SaveError, SaveOptions, Settings, save_with,
};
use egui::Ui;

use super::{
    Event, FileCommand, Files, Intent, Opened, Opening, Origin, Output, Purpose,
    internal_load_error, menu_item, submenu_label, with_extension,
};
use crate::{
    commands::{Command, CommandFrame},
    icons,
    model::{Action, Model, Notice, display_name},
    widgets,
};

const FOLDER: &str = "templates";
const DEFAULT_TEMPLATE_KEY: &str = "files.default_template";
pub const NEW_FROM_TEMPLATE: &str = "New from template";
const NO_TEMPLATES: &str = "No templates yet";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Templates {
    folder: Option<PathBuf>,
    listed: Vec<PathBuf>,
}

impl Templates {
    pub fn in_config(config_dir: Option<&Path>) -> Self {
        Self {
            folder: config_dir.map(|dir| dir.join(FOLDER)),
            listed: Vec::new(),
        }
    }

    pub fn folder(&self) -> Option<&Path> {
        self.folder.as_deref()
    }

    pub fn listed(&self) -> &[PathBuf] {
        &self.listed
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.listed
            .iter()
            .filter_map(|path| path.file_name().and_then(OsStr::to_str))
    }

    pub fn lists(&self, name: &str) -> bool {
        self.names().any(|listed| listed == name)
    }

    pub fn path_of(&self, name: &str) -> Option<PathBuf> {
        let plain = Path::new(name).file_name() == Some(OsStr::new(name));
        plain
            .then(|| self.folder.as_ref().map(|folder| folder.join(name)))
            .flatten()
    }
}

pub fn default_template(settings: &Settings) -> Option<String> {
    settings
        .text(DEFAULT_TEMPLATE_KEY)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}

pub fn write_default_template(name: Option<&str>, settings: &mut Settings) {
    match name {
        Some(name) => settings.set_text(DEFAULT_TEMPLATE_KEY, name),
        None => settings.remove(DEFAULT_TEMPLATE_KEY),
    }
}

pub fn title(name: &str) -> String {
    Path::new(name).file_stem().map_or_else(
        || name.to_owned(),
        |stem| stem.to_string_lossy().into_owned(),
    )
}

pub fn list(folder: &Path) -> Vec<PathBuf> {
    let entries = match fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Vec::new(),
        Err(error) => {
            log::warn!(
                "could not list the templates in {}: {error}",
                folder.display()
            );
            return Vec::new();
        }
    };
    let mut listed: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| is_template(path))
        .collect();
    listed.sort_by_cached_key(|path| display_name(Some(path)).to_lowercase());
    listed
}

fn is_template(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case(FILE_EXTENSION))
        && path.is_file()
}

pub fn save(document: &Document, path: &Path) -> Result<(), SaveError> {
    save_with(document, path, &SaveOptions::default()).map(|_| ())
}

impl Files {
    pub fn templates(&self) -> &Templates {
        &self.templates
    }

    pub(super) fn read_default_template(&mut self, settings: &Settings) {
        self.default_template = default_template(settings);
    }

    pub(super) fn list_templates(&mut self, then: Option<Purpose>, model: &Model) {
        let Some(folder) = self.templates.folder.clone() else {
            if let Some(purpose) = then {
                self.pick(purpose, model);
            }
            return;
        };
        self.spawn(
            move || {
                if then.is_some()
                    && let Err(error) = fs::create_dir_all(&folder)
                {
                    log::warn!(
                        "could not make the templates folder {}: {error}",
                        folder.display()
                    );
                }
                Event::TemplatesListed {
                    listed: list(&folder),
                    then,
                }
            },
            move || Event::TemplatesListed {
                listed: Vec::new(),
                then,
            },
        );
    }

    pub(super) fn templates_listed(
        &mut self,
        listed: Vec<PathBuf>,
        then: Option<Purpose>,
        model: &Model,
    ) {
        self.templates.listed = listed;
        if let Some(purpose) = then {
            self.pick(purpose, model);
        }
    }

    pub(super) fn new_model(&mut self, model: &mut Model) {
        let Some(name) = self.default_template.clone() else {
            model.replace(Document::default(), None, None, false);
            return;
        };
        match self.templates.path_of(&name) {
            Some(path) => self.load_template(path, Origin::DefaultTemplate, model),
            None => {
                model.replace(Document::default(), None, None, false);
                model.perform(Action::Inform(Notice::failure(format!(
                    "Could not start from the template “{}”: caditor has no templates folder. \
                     A new empty model was started instead.",
                    title(&name)
                ))));
            }
        }
    }

    pub(super) fn load_template(&mut self, path: PathBuf, origin: Origin, model: &mut Model) {
        self.abandon_open();
        let opening = Opening::new(path.clone(), origin);
        let cancel = opening.token();
        self.opening = Some(opening);
        self.open_attempt += 1;
        let attempt = self.open_attempt;
        let revision = model.revision();
        let loader = Arc::clone(&self.model_loader);
        let failed = path.clone();
        self.spawn_own(
            "open",
            move || {
                let result = loader(&path, &cancel);
                Event::TemplateLoaded {
                    path,
                    origin,
                    revision,
                    attempt,
                    result,
                }
            },
            move || Event::TemplateLoaded {
                path: failed,
                origin,
                revision,
                attempt,
                result: Err(internal_load_error()),
            },
        );
    }

    pub(super) fn template_loaded(
        &mut self,
        path: PathBuf,
        origin: Origin,
        revision: u64,
        result: Result<Loaded, LoadError>,
        model: &mut Model,
    ) {
        let changed_meanwhile = model.revision() != revision && model.is_dirty();
        match result {
            Ok(loaded) => {
                let opened = Opened {
                    path,
                    loaded,
                    notes: Vec::new(),
                    origin,
                };
                if changed_meanwhile {
                    self.guard = Some(Intent::Replace(Box::new(opened)));
                } else {
                    self.finish_open(opened, model);
                }
            }
            Err(LoadError::Cancelled) => {}
            Err(error) => {
                let name = display_name(Some(&path));
                let next = if origin == Origin::DefaultTemplate {
                    " Choose another default template in Preferences, or save one again with \
                     Save as template…."
                } else {
                    ""
                };
                if changed_meanwhile {
                    model.set_notice(Notice::failure(format!(
                        "Could not start from the template “{name}”: {error}. Your model was not \
                         changed.{next}"
                    )));
                    return;
                }
                model.replace(Document::default(), None, None, false);
                model.set_notice(Notice::failure(format!(
                    "Could not start from the template “{name}”: {error}. A new empty model was \
                     started instead.{next}"
                )));
            }
        }
    }

    pub(super) fn template_target(&mut self, path: PathBuf, model: &mut Model) {
        let named = with_extension(path.clone());
        if model.path() == Some(named.as_path()) || model.path() == Some(path.as_path()) {
            model.set_notice(Notice::info(format!(
                "“{}” is the open model's own file, so it is already saved as that template \
                 whenever you save.",
                display_name(Some(&named))
            )));
            return;
        }
        self.check_output(Output::Template { path });
    }

    pub(super) fn save_template(&mut self, path: PathBuf, model: &Model) {
        let document = model.document().clone();
        let failed = path.clone();
        self.spawn(
            move || Event::TemplateSaved {
                result: save(&document, &path),
                path,
            },
            move || Event::TemplateSaved {
                path: failed,
                result: Err(SaveError::Unconvertible),
            },
        );
    }

    pub(super) fn template_saved(
        &mut self,
        path: &Path,
        result: Result<(), SaveError>,
        model: &mut Model,
    ) {
        let name = display_name(Some(path));
        match result {
            Ok(()) => {
                model.set_notice(Notice::info(format!(
                    "Saved “{name}” as a template. New from template starts a model from it, and \
                     Preferences can make New model start from it."
                )));
                self.list_templates(None, model);
            }
            Err(error) => model.set_notice(Notice::failure(format!(
                "Could not save the template “{name}”: {error}."
            ))),
        }
    }
}

pub fn menu(
    ui: &mut Ui,
    files: &Files,
    commands: &CommandFrame<'_>,
    chosen: &mut Vec<Command>,
    actions: &mut Vec<Action>,
) {
    let response = ui.menu_button(
        submenu_label(ui, icons::TEMPLATE, NEW_FROM_TEMPLATE),
        |ui| {
            widgets::fitted_menu(ui, |ui| {
                for path in files.templates.listed() {
                    let name = title(&display_name(Some(path)));
                    let item = widgets::menu_item(ui, icons::TEMPLATE, &name, None).on_hover_text(
                        format!("Start a new untitled model as a copy of {}", path.display()),
                    );
                    if item.clicked() {
                        actions.push(Action::File(FileCommand::NewFromTemplate(Some(
                            path.clone(),
                        ))));
                    }
                }
                if files.templates.listed().is_empty() {
                    ui.label(widgets::muted(NO_TEMPLATES, ui));
                }
                ui.separator();
                for command in [Command::NewFromTemplate, Command::SaveAsTemplate] {
                    if menu_item(ui, commands, command).clicked() {
                        chosen.push(command);
                    }
                }
            });
        },
    );
    widgets::named(response.response, NEW_FROM_TEMPLATE);
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use caditor_file::Settings;

    use super::{Templates, default_template, title, write_default_template};

    #[test]
    fn a_default_template_names_a_file_inside_the_templates_folder() {
        let templates = Templates::in_config(Some(Path::new("config")));

        assert_eq!(
            templates.path_of("Base.caditor"),
            Some(Path::new("config").join("templates").join("Base.caditor"))
        );
        assert_eq!(templates.path_of("../Base.caditor"), None);
        assert_eq!(templates.path_of(""), None);
        assert_eq!(Templates::in_config(None).path_of("Base.caditor"), None);
        assert_eq!(title("Base.caditor"), "Base");
    }

    #[test]
    fn the_default_template_round_trips_through_the_settings() {
        let mut settings = Settings::default();

        write_default_template(Some("Base.caditor"), &mut settings);

        assert_eq!(default_template(&settings).as_deref(), Some("Base.caditor"));

        write_default_template(None, &mut settings);

        assert_eq!(default_template(&settings), None);
    }
}
