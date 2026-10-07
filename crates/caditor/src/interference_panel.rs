use std::sync::Arc;

use caditor_document::{Document, FeatureId};
use caditor_geometry::Point3;
use egui::{ScrollArea, Ui};

use crate::{
    appearance::{SPACE_M, SPACE_S},
    bodies::MassAccuracy,
    icons,
    interference::{Bodies, Finding, InterferenceTool, Pair, Report, Scope},
    layout,
    measure_panel::{self, Card, Row},
    model::Model,
    selection::Selection,
    units::LengthUnit,
    widgets::{self, Tone},
};

pub const TITLE: &str = "Interference";
pub const CLOSE: &str = "Close the interference panel";
pub const COPY_ALL: &str = "Copy all";
pub const SHOW_PLACE: &str = "Show where";
pub const NONE_FOUND: &str = "No bodies overlap or touch.";
pub const TOO_FEW: &str = "Interference is checked between two bodies or more; this model shows \
                           fewer.";
pub const UNCHECKED: &str = "Whether these bodies overlap could not be worked out where their \
                             faces meet. Measure between them, or move one slightly and check \
                             again.";
pub const EVERYTHING: &str = "Every body shown, pair by pair. Select faces, edges or vertices of \
                              bodies to check those alone.";
pub const CHOSEN: &str = "The selected bodies, pair by pair.";
pub const MAX_LISTED: usize = 50;
const UNMESHED: &str = "The shared volume could not be meshed, so its size is not known.";
const PANEL_WIDTH: f32 = 300.0;
const MIN_PANEL_WIDTH: f32 = 220.0;
const APPROXIMATELY: &str = "≈ ";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkKind {
    Overlap,
    Touch,
    Unchecked,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Mark {
    pub place: Point3,
    pub label: String,
    pub kind: MarkKind,
    pub outline: Option<Arc<Vec<Vec<Point3>>>>,
}

fn body_name(document: &Document, body: FeatureId) -> String {
    document.feature(body).map_or_else(
        || "A deleted body".to_owned(),
        |feature| feature.name.clone(),
    )
}

fn pair_name(document: &Document, pair: Pair) -> String {
    format!(
        "{} and {}",
        body_name(document, pair.first),
        body_name(document, pair.second)
    )
}

pub fn scope_text(document: &Document, bodies: &Bodies) -> String {
    match bodies.scope {
        Scope::Everything => EVERYTHING.to_owned(),
        Scope::Chosen => CHOSEN.to_owned(),
        Scope::AgainstTheRest(body) => format!(
            "{} against every other body shown. Select none to check every pair.",
            body_name(document, body)
        ),
    }
}

fn counted(count: usize, one: &str, several: &str) -> String {
    if count == 1 {
        format!("1 pair {one}")
    } else {
        format!("{count} pairs {several}")
    }
}

pub fn summary(report: &Report) -> Option<(Tone, String)> {
    if report.bodies.count < 2 {
        return Some((Tone::Info, TOO_FEW.to_owned()));
    }
    if report.is_checking() {
        return None;
    }
    let count = |wanted: fn(&Finding) -> bool| {
        report
            .contacts()
            .filter(|(_, finding)| wanted(finding))
            .count()
    };
    let overlaps = count(|finding| matches!(finding, Finding::Overlapping(_)));
    let touches = count(|finding| matches!(finding, Finding::Touching(_)));
    let unchecked = count(|finding| matches!(finding, Finding::Unchecked(_)));
    let parts: Vec<String> = [
        (overlaps > 0).then(|| counted(overlaps, "overlaps", "overlap")),
        (touches > 0).then(|| counted(touches, "touches", "touch")),
        (unchecked > 0).then(|| counted(unchecked, "could not be checked", "could not be checked")),
    ]
    .into_iter()
    .flatten()
    .collect();
    let tone = if overlaps > 0 {
        Tone::Error
    } else if unchecked > 0 {
        Tone::Warning
    } else if touches > 0 {
        Tone::Info
    } else {
        return Some((Tone::Success, NONE_FOUND.to_owned()));
    };
    let text = match parts.as_slice() {
        [only] => format!("{only}."),
        [first, second] => format!("{first} and {second}."),
        [first, second, third] => format!("{first}, {second} and {third}."),
        _ => String::new(),
    };
    Some((tone, text))
}

pub fn progress(report: &Report) -> Option<String> {
    report.is_checking().then(|| {
        format!(
            "Checking… {} of {} pairs",
            report.checked(),
            report.checked() + report.pending
        )
    })
}

fn approximately(text: String, approximate: bool) -> String {
    if approximate {
        format!("{APPROXIMATELY}{text}")
    } else {
        text
    }
}

fn rank(finding: &Finding) -> (u8, u64) {
    match finding {
        Finding::Overlapping(overlap) => (
            0,
            u64::MAX
                - overlap
                    .mass
                    .map_or(0, |mass| mass.properties.volume.abs().to_bits()),
        ),
        Finding::Unchecked(_) => (1, 0),
        Finding::Touching(_) => (2, 0),
        Finding::Apart => (3, 0),
    }
}

fn ranked(report: &Report) -> Vec<&(Pair, Finding)> {
    let mut contacts: Vec<&(Pair, Finding)> = report.contacts().collect();
    contacts.sort_by_key(|(pair, finding)| (rank(finding), *pair));
    contacts
}

fn card_of(document: &Document, pair: Pair, finding: &Finding, unit: LengthUnit) -> Card {
    let title = pair_name(document, pair);
    let position = |point: Point3| unit.measured_position([point.x, point.y, point.z]);
    match finding {
        Finding::Overlapping(overlap) => match overlap.mass {
            Some(mass) => {
                let (approximate, notes) = match mass.accuracy {
                    MassAccuracy::Exact => (false, Vec::new()),
                    MassAccuracy::Mesh {
                        chord,
                        volume_within,
                    } => (
                        true,
                        vec![(
                            Tone::Info,
                            format!(
                                "Found from a mesh following curved faces within {}; the volume \
                                 is within {}.",
                                unit.measured_length(chord),
                                unit.measured_volume(volume_within)
                            ),
                        )],
                    ),
                };
                let mut rows = vec![Row {
                    label: "Shared volume".to_owned(),
                    text: approximately(unit.measured_volume(mass.properties.volume), approximate),
                }];
                if let Some(size) = mass.size {
                    rows.push(Row {
                        label: "Size".to_owned(),
                        text: approximately(unit.measured_size(size), approximate),
                    });
                }
                rows.push(Row {
                    label: "Centroid".to_owned(),
                    text: approximately(position(mass.properties.centroid), approximate),
                });
                Card { title, rows, notes }
            }
            None => Card {
                title,
                rows: vec![Row {
                    label: "Near".to_owned(),
                    text: position(overlap.place),
                }],
                notes: vec![(Tone::Neutral, UNMESHED.to_owned())],
            },
        },
        Finding::Touching(place) => Card {
            title,
            rows: vec![Row {
                label: "Touch at".to_owned(),
                text: position(*place),
            }],
            notes: Vec::new(),
        },
        Finding::Unchecked(_) | Finding::Apart => Card {
            title,
            rows: Vec::new(),
            notes: vec![(Tone::Warning, UNCHECKED.to_owned())],
        },
    }
}

pub fn cards(
    document: &Document,
    report: &Report,
    unit: LengthUnit,
) -> Vec<(Card, Option<Point3>)> {
    ranked(report)
        .into_iter()
        .take(MAX_LISTED)
        .map(|(pair, finding)| (card_of(document, *pair, finding, unit), finding.place()))
        .collect()
}

pub fn unlisted(report: &Report) -> usize {
    report.contacts().count().saturating_sub(MAX_LISTED)
}

pub fn marks(document: &Document, report: &Report) -> Vec<Mark> {
    ranked(report)
        .into_iter()
        .take(MAX_LISTED)
        .filter_map(|(pair, finding)| {
            let place = finding.place()?;
            let name = pair_name(document, *pair);
            let (kind, label, outline) = match finding {
                Finding::Overlapping(overlap) => (
                    MarkKind::Overlap,
                    format!("{name} overlap"),
                    Some(Arc::clone(&overlap.outline)),
                ),
                Finding::Touching(_) => (MarkKind::Touch, format!("{name} touch"), None),
                Finding::Unchecked(_) | Finding::Apart => (
                    MarkKind::Unchecked,
                    format!("{name}: not checked here"),
                    None,
                ),
            };
            Some(Mark {
                place,
                label,
                kind,
                outline,
            })
        })
        .collect()
}

pub struct InterferenceContext<'a> {
    pub model: &'a Model,
    pub selection: &'a Selection,
    pub tree_selected: Option<FeatureId>,
}

pub fn refresh(context: &InterferenceContext<'_>, tool: &mut InterferenceTool) {
    let refreshed =
        tool.interference
            .refresh(context.model, context.selection, context.tree_selected);
    if let Some(report) = refreshed {
        tool.report = Some(report);
    }
}

pub fn show(ui: &mut Ui, model: &Model, tool: &mut InterferenceTool, room: f32) -> Option<Point3> {
    let report = tool.report.as_ref()?;
    let document = model.document();
    let unit = model.length_unit();
    let cards = cards(document, report, unit);
    let unlisted = unlisted(report);
    let summary = summary(report);
    let progress = progress(report);
    let scope = scope_text(document, &report.bodies);
    let mut close = false;
    let mut shown = None;
    egui::Panel::right("interference")
        .resizable(true)
        .default_size(PANEL_WIDTH)
        .size_range(layout::panel_widths(room, MIN_PANEL_WIDTH))
        .show(ui, |ui| {
            ui.add_space(SPACE_S);
            widgets::panel_header(ui, icons::INTERFERENCE, TITLE, |ui| {
                close = widgets::icon_button(ui, icons::CLOSE, CLOSE).clicked();
                let everything = summary
                    .iter()
                    .map(|(_, text)| text.clone())
                    .chain(cards.iter().map(|(card, _)| card.text()))
                    .collect::<Vec<_>>()
                    .join("\n");
                let copy = widgets::small_button(ui, icons::COPY, COPY_ALL);
                if ui
                    .add(copy)
                    .on_hover_text("Copy every finding in the panel as text")
                    .clicked()
                {
                    ui.ctx().copy_text(everything);
                }
                if let Some(progress) = &progress {
                    ui.label(widgets::muted(progress, ui));
                }
            });
            ui.add_space(SPACE_S);
            ScrollArea::vertical().show(ui, |ui| {
                ui.label(widgets::muted(&scope, ui));
                ui.add_space(SPACE_S);
                if let Some((tone, text)) = &summary {
                    let response = widgets::callout(ui, *tone, |ui| ui.label(text));
                    widgets::announced(ui, &response, *tone == Tone::Error);
                    ui.add_space(SPACE_M);
                }
                for (index, (card, place)) in cards.iter().enumerate() {
                    measure_panel::show_card(ui, ("interference", index), card);
                    if let Some(place) = place {
                        let button = widgets::small_button(ui, icons::SHOW_PLACE, SHOW_PLACE);
                        if ui.add(button).clicked() {
                            shown = Some(*place);
                        }
                    }
                    ui.add_space(SPACE_M);
                }
                if unlisted > 0 {
                    ui.label(widgets::muted(
                        format!(
                            "{unlisted} more not listed. Select bodies to check fewer at a time."
                        ),
                        ui,
                    ));
                }
            });
        });
    if close {
        tool.toggle();
    }
    shown
}
