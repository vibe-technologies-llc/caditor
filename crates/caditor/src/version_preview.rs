use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use caditor_document::{
    CancelToken, Document, Evaluation, FeatureId, FeatureState, ModelEvaluator, Recompute,
};
use caditor_expression::ParameterId;
use caditor_file::{LoadError, SavedState, load_version_cancellable};
use caditor_render::{ImageError, SurfaceSize};
use egui::{
    ColorImage, Id, Image, Sense, TextureHandle, TextureOptions, Ui, Vec2, load::SizedTexture,
};
use parking_lot::Mutex;

use crate::{
    appearance,
    history::{self, HistoryCommand},
    image_export::ReadPixels,
    snapshot::{self, Snapshot, Unmeshed},
    widgets::{self, Tone},
};

pub const PICTURE_SIZE: SurfaceSize = SurfaceSize {
    width: 480,
    height: 300,
};
const SHOWN_PICTURE: Vec2 = Vec2::new(240.0, 150.0);
const MAX_NAMED: usize = 4;
const PICTURE_NAME: &str = "Picture of the bodies in this version";
pub const CANCEL: &str = "Cancel the preview";
pub const HIDE: &str = "Hide preview";
pub const SHOW: &str = "Preview";
pub const SAME_AS_NOW: &str = "The same as the model now.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Stage {
    #[default]
    Reading,
    Recomputing {
        done: usize,
        total: usize,
    },
    Meshing,
}

impl Stage {
    fn text(self) -> String {
        match self {
            Self::Reading => "Reading the version…".to_owned(),
            Self::Recomputing { done, total } => {
                format!("Recomputing the version: {done} of {total} features…")
            }
            Self::Meshing => "Meshing its bodies…".to_owned(),
        }
    }
}

#[derive(Debug, Default)]
pub struct Progress(Mutex<Stage>);

impl Progress {
    fn set(&self, stage: Stage) {
        *self.0.lock() = stage;
    }

    fn stage(&self) -> Stage {
        *self.0.lock()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PreviewFailure {
    #[error("This version could not be read: {0}.")]
    Unreadable(LoadError),
    #[error("caditor ran into an internal error while preparing this preview.")]
    Crashed,
    #[error("The preview was cancelled.")]
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PictureFailure {
    #[error("Its bodies could not be meshed for a picture: {0}.")]
    Unmeshed(Unmeshed),
    #[error("The picture could not be drawn: {0}.")]
    Drawing(ImageError),
    #[error("caditor ran into an internal error while reading the picture back.")]
    Crashed,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Summary {
    pub features: usize,
    pub bodies: usize,
    pub failed: Vec<String>,
    pub issues: Vec<String>,
}

impl Summary {
    pub fn of(document: &Document, evaluation: &Evaluation, issues: Vec<String>) -> Self {
        let failed = document
            .features()
            .filter(|feature| {
                evaluation
                    .feature(feature.id())
                    .is_some_and(|status| matches!(status.state, FeatureState::Failed(_)))
            })
            .map(|feature| feature.name.clone())
            .collect();
        Self {
            features: document.features().len(),
            bodies: built_bodies(evaluation),
            failed,
            issues,
        }
    }

    fn headline(&self) -> String {
        format!(
            "{} · {}",
            counted(self.features, "feature", "features"),
            counted(self.bodies, "body", "bodies")
        )
    }
}

fn built_bodies(evaluation: &Evaluation) -> usize {
    evaluation
        .bodies()
        .filter(|(body, _)| evaluation.body(*body).is_some())
        .count()
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Changes {
    pub only_in_version: Vec<String>,
    pub added_since: Vec<String>,
    pub changed_since: Vec<String>,
    pub parameters: Vec<String>,
}

impl Changes {
    pub fn between(version: &Document, now: &Document) -> Self {
        let now_features: BTreeMap<FeatureId, _> = now
            .features()
            .map(|feature| (feature.id(), feature))
            .collect();
        let version_features: BTreeSet<FeatureId> =
            version.features().map(|feature| feature.id()).collect();

        let only_in_version = version
            .features()
            .filter(|feature| !now_features.contains_key(&feature.id()))
            .map(|feature| feature.name.clone())
            .collect();
        let changed_since = version
            .features()
            .filter_map(|feature| {
                let current = now_features.get(&feature.id())?;
                if current.same_content(feature) {
                    return None;
                }
                Some(if current.name == feature.name {
                    feature.name.clone()
                } else {
                    format!("{} (now “{}”)", feature.name, current.name)
                })
            })
            .collect();
        let added_since = now
            .features()
            .filter(|feature| !version_features.contains(&feature.id()))
            .map(|feature| feature.name.clone())
            .collect();

        Self {
            only_in_version,
            added_since,
            changed_since,
            parameters: changed_parameters(version, now),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.only_in_version.is_empty()
            && self.added_since.is_empty()
            && self.changed_since.is_empty()
            && self.parameters.is_empty()
    }

    pub fn lines(&self) -> Vec<String> {
        [
            ("Not in the model now", &self.only_in_version),
            ("Added since", &self.added_since),
            ("Changed since", &self.changed_since),
            ("Parameters changed since", &self.parameters),
        ]
        .into_iter()
        .filter(|(_, names)| !names.is_empty())
        .map(|(heading, names)| format!("{heading}: {}", listed(names)))
        .collect()
    }
}

fn changed_parameters(version: &Document, now: &Document) -> Vec<String> {
    let now_parameters: BTreeMap<ParameterId, _> = now
        .parameters()
        .iter()
        .map(|parameter| (parameter.id(), parameter))
        .collect();
    let version_ids: BTreeSet<ParameterId> = version
        .parameters()
        .iter()
        .map(|parameter| parameter.id())
        .collect();

    let differing = version.parameters().iter().filter(|parameter| {
        now_parameters
            .get(&parameter.id())
            .is_none_or(|current| current != parameter)
    });
    let added = now
        .parameters()
        .iter()
        .filter(|parameter| !version_ids.contains(&parameter.id()));
    let mut names: Vec<String> = differing
        .chain(added)
        .map(|parameter| parameter.name.clone())
        .collect();
    names.dedup();
    names
}

fn listed(names: &[String]) -> String {
    let shown = names
        .iter()
        .take(MAX_NAMED)
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(", ");
    match names.len().saturating_sub(MAX_NAMED) {
        0 => shown,
        more => format!("{shown} and {more} more"),
    }
}

fn counted(count: usize, one: &str, many: &str) -> String {
    match count {
        1 => format!("1 {one}"),
        count => format!("{count} {many}"),
    }
}

pub struct Prepared {
    pub document: Document,
    pub summary: Summary,
    pub picture: Result<Option<Snapshot>, PictureFailure>,
}

pub fn prepare(
    path: &Path,
    index: usize,
    progress: &Progress,
    cancel: &CancelToken,
) -> Result<Prepared, PreviewFailure> {
    progress.set(Stage::Reading);
    let loaded = load_version_cancellable(path, index, cancel).map_err(|error| match error {
        LoadError::Cancelled => PreviewFailure::Cancelled,
        error => PreviewFailure::Unreadable(error),
    })?;

    let evaluation =
        Recompute::default().run(&loaded.document, &ModelEvaluator, cancel, &|done, total| {
            progress.set(Stage::Recomputing { done, total })
        });
    if cancel.is_cancelled() {
        return Err(PreviewFailure::Cancelled);
    }
    let summary = Summary::of(&loaded.document, &evaluation, loaded.issues);

    let picture = if summary.bodies == 0 {
        Ok(None)
    } else {
        progress.set(Stage::Meshing);
        match snapshot::take_unless(&loaded.document, &evaluation, PICTURE_SIZE, cancel) {
            Ok(snapshot) => Ok(Some(snapshot)),
            Err(Unmeshed::Cancelled) => return Err(PreviewFailure::Cancelled),
            Err(unmeshed) => Err(PictureFailure::Unmeshed(unmeshed)),
        }
    };
    Ok(Prepared {
        document: loaded.document,
        summary,
        picture,
    })
}

pub enum Picture {
    NoBodies,
    Waiting(Box<Snapshot>),
    Drawing,
    Drawn(Arc<ColorImage>),
    Failed(PictureFailure),
}

pub struct Ready {
    document: Document,
    summary: Summary,
    changes: Option<Changes>,
    compared: Option<(u64, u64)>,
    picture: Picture,
}

pub enum State {
    Working {
        stopped: Arc<AtomicBool>,
        progress: Arc<Progress>,
    },
    Ready(Box<Ready>),
    Failed(PreviewFailure),
}

pub struct Preview {
    pub index: usize,
    pub saved: SavedState,
    pub ticket: u64,
    state: State,
}

pub struct Started {
    pub ticket: u64,
    pub cancel: CancelToken,
    pub progress: Arc<Progress>,
}

impl Preview {
    pub fn start(index: usize, saved: SavedState, ticket: u64) -> (Self, Started) {
        let stopped = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(Progress::default());
        let flag = Arc::clone(&stopped);
        let started = Started {
            ticket,
            cancel: CancelToken::new(move || flag.load(Ordering::SeqCst)),
            progress: Arc::clone(&progress),
        };
        let preview = Self {
            index,
            saved,
            ticket,
            state: State::Working { stopped, progress },
        };
        (preview, started)
    }

    pub fn stop(&self) {
        if let State::Working { stopped, .. } = &self.state {
            stopped.store(true, Ordering::SeqCst);
        }
    }

    pub fn prepared(&mut self, result: Result<Prepared, PreviewFailure>) {
        self.state = match result {
            Ok(prepared) => State::Ready(Box::new(Ready {
                document: prepared.document,
                summary: prepared.summary,
                changes: None,
                compared: None,
                picture: match prepared.picture {
                    Ok(Some(snapshot)) => Picture::Waiting(Box::new(snapshot)),
                    Ok(None) => Picture::NoBodies,
                    Err(failure) => Picture::Failed(failure),
                },
            })),
            Err(failure) => State::Failed(failure),
        };
    }

    pub fn compare(&mut self, now: &Document, same_file: bool, stamp: (u64, u64)) {
        let State::Ready(ready) = &mut self.state else {
            return;
        };
        if !same_file {
            ready.changes = None;
            ready.compared = None;
            return;
        }
        if ready.compared != Some(stamp) {
            ready.changes = Some(Changes::between(&ready.document, now));
            ready.compared = Some(stamp);
        }
    }

    pub fn waiting_picture(&self) -> Option<&Snapshot> {
        match &self.state {
            State::Ready(ready) => match &ready.picture {
                Picture::Waiting(snapshot) => Some(snapshot),
                _ => None,
            },
            _ => None,
        }
    }

    pub fn set_picture(&mut self, picture: Picture) {
        if let State::Ready(ready) = &mut self.state {
            ready.picture = picture;
        }
    }

    #[cfg(test)]
    pub fn changes(&self) -> Option<&Changes> {
        match &self.state {
            State::Ready(ready) => ready.changes.as_ref(),
            _ => None,
        }
    }

    #[cfg(test)]
    pub fn summary(&self) -> Option<&Summary> {
        match &self.state {
            State::Ready(ready) => Some(&ready.summary),
            _ => None,
        }
    }

    #[cfg(test)]
    pub fn is_drawn(&self) -> bool {
        matches!(&self.state, State::Ready(ready) if matches!(ready.picture, Picture::Drawn(_)))
    }
}

pub fn read_picture(mut rows: ReadPixels) -> Result<Arc<ColorImage>, PictureFailure> {
    let size = SurfaceSize {
        width: rows.width(),
        height: rows.height(),
    };
    let mut pixels = Vec::new();
    while let Some(band) = rows.next_rows() {
        pixels.extend_from_slice(band.map_err(PictureFailure::Drawing)?);
    }
    drawn(size, &pixels).ok_or(PictureFailure::Drawing(ImageError::Readback))
}

fn drawn(size: SurfaceSize, pixels: &[u8]) -> Option<Arc<ColorImage>> {
    let expected = size.width as usize * size.height as usize * 4;
    (pixels.len() == expected).then(|| {
        Arc::new(ColorImage::from_rgba_unmultiplied(
            [size.width as usize, size.height as usize],
            pixels,
        ))
    })
}

pub fn show(ui: &mut Ui, preview: &Preview) -> Option<HistoryCommand> {
    let mut command = None;
    widgets::card(ui, |ui| {
        ui.label(widgets::strong(format!(
            "Preview of the version saved {}",
            history::when_saved(&preview.saved)
        )));
        match &preview.state {
            State::Working { progress, .. } => {
                ui.horizontal_wrapped(|ui| {
                    widgets::spinner(ui);
                    ui.label(progress.stage().text());
                    if ui
                        .add(widgets::button("Cancel"))
                        .on_hover_text(CANCEL)
                        .clicked()
                    {
                        command = Some(HistoryCommand::StopPreview);
                    }
                });
            }
            State::Failed(failure) => {
                widgets::callout(ui, Tone::Error, |ui| {
                    ui.label(failure.to_string());
                    ui.label("Restoring it would fail the same way; the other versions are not affected.");
                });
            }
            State::Ready(ready) => {
                ui.horizontal_wrapped(|ui| {
                    ui.vertical(|ui| picture(ui, &ready.picture, preview.ticket));
                    ui.vertical(|ui| {
                        ui.set_max_width(ui.available_width().max(SHOWN_PICTURE.x));
                        details(ui, ready);
                        if ui
                            .add(widgets::button("Restore this version"))
                            .on_hover_text("Bring the model back to this version; Undo reverses it")
                            .clicked()
                        {
                            command = Some(HistoryCommand::Restore(preview.index));
                        }
                    });
                });
            }
        }
    });
    command
}

fn details(ui: &mut Ui, ready: &Ready) {
    ui.label(ready.summary.headline());
    if !ready.summary.failed.is_empty() {
        widgets::callout(ui, Tone::Warning, |ui| {
            ui.label(format!(
                "{} in this version: {}.",
                counted(ready.summary.failed.len(), "feature fails", "features fail"),
                listed(&ready.summary.failed)
            ));
        });
    }
    if !ready.summary.issues.is_empty() {
        widgets::callout(ui, Tone::Warning, |ui| {
            ui.label(format!(
                "Parts of this version could not be read: {}",
                listed(&ready.summary.issues)
            ));
        });
    }
    match &ready.changes {
        Some(changes) if changes.is_empty() => {
            ui.label(SAME_AS_NOW);
        }
        Some(changes) => {
            ui.label(widgets::muted("Compared with the model now:", ui));
            for line in changes.lines() {
                ui.label(line);
            }
        }
        None => {
            ui.label(widgets::muted(
                "Another model is open, so this version is not compared with it.",
                ui,
            ));
        }
    }
}

fn picture(ui: &mut Ui, picture: &Picture, ticket: u64) {
    match picture {
        Picture::Drawn(image) => {
            let texture = texture(ui, image, ticket);
            ui.add(
                Image::new(SizedTexture::new(texture.id(), SHOWN_PICTURE)).alt_text(PICTURE_NAME),
            );
        }
        Picture::Waiting(_) | Picture::Drawing => placeholder(ui, |ui| {
            ui.horizontal(|ui| {
                widgets::spinner(ui);
                ui.label("Drawing…");
            });
        }),
        Picture::NoBodies => placeholder(ui, |ui| {
            ui.label(widgets::muted("No bodies to show", ui));
        }),
        Picture::Failed(failure) => placeholder(ui, |ui| {
            let muted = appearance::tokens(ui).text_muted;
            ui.horizontal_wrapped(|ui| {
                widgets::icon_label(ui, Tone::Warning.icon(), muted);
                ui.label(widgets::muted(failure.to_string(), ui));
            });
        }),
    }
}

fn placeholder(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    let (rect, _) = ui.allocate_exact_size(SHOWN_PICTURE, Sense::hover());
    let tokens = appearance::tokens(ui);
    let radius = f32::from(appearance::WIDGET_RADIUS);
    ui.painter().rect_filled(rect, radius, tokens.sunken);
    let mut inner = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink(radius)).layout(
        egui::Layout::centered_and_justified(egui::Direction::TopDown),
    ));
    add(&mut inner);
}

fn texture(ui: &Ui, image: &Arc<ColorImage>, ticket: u64) -> TextureHandle {
    let id = Id::new("version-preview-picture");
    if let Some((shown, texture)) = ui.data(|data| data.get_temp::<(u64, TextureHandle)>(id))
        && shown == ticket
    {
        return texture;
    }
    let texture = ui.ctx().load_texture(
        "version-preview",
        ColorImage::clone(image),
        TextureOptions::LINEAR,
    );
    ui.data_mut(|data| data.insert_temp(id, (ticket, texture.clone())));
    texture
}

#[cfg(test)]
mod tests {
    use caditor_document::{Edit, Transaction};

    use super::*;
    use crate::samples::Sample;

    fn plate() -> Document {
        Sample::Plate.document().unwrap()
    }

    fn with_width(document: &Document, width: &str) -> Document {
        let mut changed = document.clone();
        let id = changed.parameter_named("width").unwrap().id();
        let expression = changed.parse(width).unwrap();
        changed
            .apply(Transaction::single(
                "Width",
                Edit::SetParameterExpression { id, expression },
            ))
            .unwrap();
        changed
    }

    #[test]
    fn a_version_is_compared_with_the_model_feature_by_feature() {
        let version = plate();
        let mut now = version.clone();

        assert!(Changes::between(&version, &now).is_empty());

        let features: Vec<_> = now
            .features()
            .map(|feature| (feature.id(), feature.name.clone()))
            .collect();
        let (last, last_name) = features.last().cloned().unwrap();
        let (first, first_name) = features.first().cloned().unwrap();
        now = with_width(&now, "61 mm");
        now.apply(Transaction::new(
            "Edit",
            vec![
                Edit::RemoveFeature { id: last },
                Edit::RenameFeature {
                    id: first,
                    name: "Base".to_owned(),
                },
            ],
        ))
        .unwrap();

        let changes = Changes::between(&version, &now);
        assert_eq!(changes.only_in_version, [last_name.as_str()]);
        assert_eq!(changes.added_since, Vec::<String>::new());
        assert_eq!(
            changes.changed_since,
            [format!("{first_name} (now “Base”)")]
        );
        assert_eq!(changes.parameters, ["width"]);
        assert_eq!(
            changes.lines(),
            [
                format!("Not in the model now: {last_name}"),
                format!("Changed since: {first_name} (now “Base”)"),
                "Parameters changed since: width".to_owned(),
            ]
        );

        let reverse = Changes::between(&now, &version);
        assert_eq!(reverse.added_since, [last_name]);
        assert!(reverse.only_in_version.is_empty());
    }

    #[test]
    fn long_lists_of_names_are_cut_and_counted() {
        let names: Vec<String> = (1..=6).map(|index| format!("Sketch {index}")).collect();

        assert_eq!(
            listed(&names),
            "Sketch 1, Sketch 2, Sketch 3, Sketch 4 and 2 more"
        );
        assert_eq!(listed(&names[..2]), "Sketch 1, Sketch 2");
        assert_eq!(counted(1, "body", "bodies"), "1 body");
        assert_eq!(counted(3, "body", "bodies"), "3 bodies");
    }

    #[test]
    fn a_version_is_read_and_recomputed_with_a_picture_ready_to_draw() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("plate.caditor");
        let document = plate();
        caditor_file::save(&document, &path, false).unwrap();
        let changed = with_width(&document, "70 mm");
        caditor_file::save(&changed, &path, false).unwrap();

        let progress = Progress::default();
        let prepared = prepare(&path, 0, &progress, &CancelToken::never()).unwrap();

        assert_eq!(prepared.summary.features, document.features().len());
        assert!(prepared.summary.bodies > 0);
        assert!(prepared.summary.failed.is_empty());
        assert!(matches!(prepared.picture, Ok(Some(_))));
        assert_eq!(progress.stage(), Stage::Meshing);
        assert_eq!(
            Changes::between(&prepared.document, &changed).parameters,
            ["width"]
        );

        let cancelled = prepare(&path, 0, &progress, &CancelToken::new(|| true));
        assert_eq!(cancelled.err(), Some(PreviewFailure::Cancelled));
        assert!(matches!(
            prepare(&path, 7, &progress, &CancelToken::never()),
            Err(PreviewFailure::Unreadable(LoadError::VersionUnavailable))
        ));
    }
}
