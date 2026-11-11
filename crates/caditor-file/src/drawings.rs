use std::collections::BTreeSet;

use caditor_document::{
    DetailParent, DetailScale, DetailView, Document, Drawing, DrawingId, DrawingScale, Drawings,
    FIRST_UNSTORABLE_ID, FeatureId, MAX_DETAIL_EXTENT, MAX_DETAIL_VIEWS, MAX_DRAWING_BODIES,
    MAX_DRAWING_NAME_CHARS, MAX_DRAWINGS, MAX_SECTION_OFFSET, MAX_SECTION_VIEWS, MIN_DETAIL_RADIUS,
    Projection, ScaleRatio, SectionPlace, SectionPlane, SectionView, SheetOrientation, SheetSize,
    StandardView, VIEW_LETTERS, drawing_name,
};
use caditor_expression::{EvalError, Expression, ParameterId};
use caditor_geometry::Point2;
use serde::{Deserialize, Serialize};

use crate::format::Lenient;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct DrawingsRecord {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub drawings: Vec<Lenient<DrawingRecord>>,
    #[serde(default)]
    pub next_id: u64,
}

fn shown() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct DrawingRecord {
    pub id: u64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub size: String,
    #[serde(default)]
    pub orientation: String,
    #[serde(default)]
    pub projection: String,
    #[serde(default = "fit")]
    pub scale: Lenient<ScaleRecord>,
    #[serde(default)]
    pub views: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bodies: Vec<u64>,
    #[serde(default = "shown")]
    pub hidden_lines: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sections: Vec<Lenient<SectionRecord>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub details: Vec<Lenient<DetailRecord>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct DetailRecord {
    pub letter: String,
    pub parent: Lenient<ParentRecord>,
    #[serde(default)]
    pub centre: [f64; 2],
    #[serde(default)]
    pub radius: f64,
    #[serde(default = "enlarged")]
    pub scale: Lenient<DetailScaleRecord>,
}

fn enlarged() -> Lenient<DetailScaleRecord> {
    Lenient::Read(DetailScaleRecord::Enlarged)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ParentRecord {
    Standard(String),
    Section(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DetailScaleRecord {
    Enlarged,
    Ratio { sheet: u32, model: u32 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct SectionRecord {
    pub letter: String,
    #[serde(default)]
    pub parent: String,
    #[serde(default)]
    pub plane: String,
    #[serde(default = "centre")]
    pub place: Lenient<PlaceRecord>,
    #[serde(default)]
    pub reversed: bool,
}

fn centre() -> Lenient<PlaceRecord> {
    Lenient::Read(PlaceRecord::Centre)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PlaceRecord {
    Centre,
    Offset(f64),
    Expression(String),
}

fn place_record(offset: &Expression) -> PlaceRecord {
    let plain = offset
        .evaluate(&|_| Err(EvalError::ParameterMissing))
        .ok()
        .map(|quantity| quantity.value)
        .filter(|value| *offset == Expression::number(*value));
    match plain {
        Some(value) => PlaceRecord::Offset(value),
        None => PlaceRecord::Expression(offset.to_stored_text()),
    }
}

fn section_record(section: &SectionView) -> SectionRecord {
    SectionRecord {
        letter: section.letter.to_string(),
        parent: section.parent.key().to_owned(),
        plane: section.plane.key().to_owned(),
        place: Lenient::Read(match &section.place {
            SectionPlace::Centre => PlaceRecord::Centre,
            SectionPlace::Offset(offset) => place_record(offset),
        }),
        reversed: section.reversed,
    }
}

fn detail_record(detail: &DetailView) -> DetailRecord {
    DetailRecord {
        letter: detail.letter.to_string(),
        parent: Lenient::Read(match detail.parent {
            DetailParent::Standard(view) => ParentRecord::Standard(view.key().to_owned()),
            DetailParent::Section(letter) => ParentRecord::Section(letter.to_string()),
        }),
        centre: [detail.centre.x, detail.centre.y],
        radius: detail.radius,
        scale: Lenient::Read(match detail.scale {
            DetailScale::Enlarged => DetailScaleRecord::Enlarged,
            DetailScale::Ratio(ratio) => DetailScaleRecord::Ratio {
                sheet: ratio.sheet,
                model: ratio.model,
            },
        }),
    }
}

fn fit() -> Lenient<ScaleRecord> {
    Lenient::Read(ScaleRecord::Fit)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ScaleRecord {
    Fit,
    Ratio { sheet: u32, model: u32 },
}

fn drawing_record(drawing: &Drawing) -> DrawingRecord {
    DrawingRecord {
        id: drawing.id.raw(),
        name: drawing.name.clone(),
        size: drawing.size.key().to_owned(),
        orientation: drawing.orientation.key().to_owned(),
        projection: drawing.projection.key().to_owned(),
        scale: Lenient::Read(match drawing.scale {
            DrawingScale::Fit => ScaleRecord::Fit,
            DrawingScale::Ratio(ratio) => ScaleRecord::Ratio {
                sheet: ratio.sheet,
                model: ratio.model,
            },
        }),
        views: drawing
            .views
            .iter()
            .map(|view| view.key().to_owned())
            .collect(),
        bodies: drawing.bodies.iter().map(|body| body.raw()).collect(),
        hidden_lines: drawing.hidden_lines,
        sections: drawing
            .sections
            .iter()
            .map(|section| Lenient::Read(section_record(section)))
            .collect(),
        details: drawing
            .details
            .iter()
            .map(|detail| Lenient::Read(detail_record(detail)))
            .collect(),
    }
}

pub(crate) fn drawings_record_of(drawings: &Drawings) -> DrawingsRecord {
    DrawingsRecord {
        drawings: drawings
            .drawings
            .iter()
            .map(|drawing| Lenient::Read(drawing_record(drawing)))
            .collect(),
        next_id: drawings.next_id,
    }
}

pub(crate) fn drawings_record(document: &Document) -> Option<DrawingsRecord> {
    let drawings = document.drawings();
    (!drawings.is_empty()).then(|| drawings_record_of(drawings))
}

fn fitting_name(name: &str, taken: &Drawings, issues: &mut Vec<String>) -> String {
    let mut name = drawing_name(name);
    if name.is_empty() {
        let numbered = taken.unused_name();
        issues.push(format!(
            "A drawing had no name, so it is called “{numbered}”."
        ));
        return numbered;
    }
    if name.chars().count() > MAX_DRAWING_NAME_CHARS {
        name = name
            .chars()
            .take(MAX_DRAWING_NAME_CHARS)
            .collect::<String>()
            .trim_end()
            .to_owned();
        issues.push(format!(
            "The name of the drawing “{name}” was longer than {MAX_DRAWING_NAME_CHARS} \
             characters, so its end was cut off."
        ));
    }
    if !taken.is_taken(&name, None) {
        return name;
    }
    let room = MAX_DRAWING_NAME_CHARS - 6;
    let base: String = name.chars().take(room).collect();
    let numbered = (2..)
        .map(|number| format!("{base} {number}"))
        .find(|candidate| !taken.is_taken(candidate, None))
        .unwrap_or_else(|| name.clone());
    issues.push(format!(
        "Two drawings were called “{name}”, so one is called “{numbered}”."
    ));
    numbered
}

fn understood<T: Copy + Default>(
    key: &str,
    read: impl Fn(&str) -> Option<T>,
    what: &str,
    name: &str,
    shown_as: impl Fn(T) -> &'static str,
    issues: &mut Vec<String>,
) -> T {
    read(key).unwrap_or_else(|| {
        let fallback = T::default();
        issues.push(format!(
            "The {what} of the drawing “{name}” was not understood, so it is {}.",
            shown_as(fallback)
        ));
        fallback
    })
}

fn restored_scale(
    record: &Lenient<ScaleRecord>,
    name: &str,
    issues: &mut Vec<String>,
) -> DrawingScale {
    let ratio = match record {
        Lenient::Read(ScaleRecord::Fit) => return DrawingScale::Fit,
        Lenient::Read(ScaleRecord::Ratio { sheet, model }) => ScaleRatio::new(*sheet, *model),
        Lenient::Unreadable(_) => {
            issues.push(format!(
                "The scale of the drawing “{name}” was not understood, so it fits the sheet."
            ));
            return DrawingScale::Fit;
        }
    };
    if ratio.is_usable() {
        return DrawingScale::Ratio(ratio);
    }
    issues.push(format!(
        "The scale {} of the drawing “{name}” cannot be used, so it fits the sheet.",
        ratio.label()
    ));
    DrawingScale::Fit
}

fn restored_views(keys: &[String], name: &str, issues: &mut Vec<String>) -> BTreeSet<StandardView> {
    let views: BTreeSet<StandardView> = keys
        .iter()
        .filter_map(|key| StandardView::from_key(key))
        .collect();
    let unknown = keys
        .iter()
        .filter(|key| StandardView::from_key(key).is_none())
        .count();
    if unknown > 0 {
        issues.push(format!(
            "{unknown} views of the drawing “{name}” were not understood, so they were left out."
        ));
    }
    views
}

fn restored_bodies(raws: &[u64], name: &str, issues: &mut Vec<String>) -> Vec<FeatureId> {
    let mut bodies: Vec<FeatureId> = raws
        .iter()
        .map(|raw| FeatureId::from_raw(*raw))
        .filter(|body| body.maker().raw() < FIRST_UNSTORABLE_ID)
        .collect();
    if bodies.len() < raws.len() {
        issues.push(format!(
            "Some bodies chosen for the drawing “{name}” could not be read, so they were left out."
        ));
    }
    if bodies.len() > MAX_DRAWING_BODIES {
        bodies.truncate(MAX_DRAWING_BODIES);
        issues.push(format!(
            "A drawing shows at most {MAX_DRAWING_BODIES} chosen bodies, so the rest of those \
             chosen for “{name}” were left out."
        ));
    }
    bodies
}

fn view_letter(record: &str, taken: impl Fn(char) -> bool) -> Option<char> {
    let mut characters = record.chars();
    let letter = characters
        .next()
        .filter(|_| characters.next().is_none())
        .filter(|letter| VIEW_LETTERS.contains(letter))
        .filter(|letter| !taken(*letter));
    letter.or_else(|| VIEW_LETTERS.into_iter().find(|letter| !taken(*letter)))
}

fn restored_place(
    record: &Lenient<PlaceRecord>,
    label: &str,
    exists: &dyn Fn(ParameterId) -> bool,
    issues: &mut Vec<String>,
) -> SectionPlace {
    match record {
        Lenient::Read(PlaceRecord::Centre) => SectionPlace::Centre,
        Lenient::Read(PlaceRecord::Offset(offset))
            if offset.is_finite() && offset.abs() <= MAX_SECTION_OFFSET =>
        {
            SectionPlace::offset(*offset)
        }
        Lenient::Read(PlaceRecord::Expression(text)) => {
            match Expression::parse_stored(text)
                .ok()
                .filter(|offset| offset.parameters().into_iter().all(exists))
            {
                Some(offset) => SectionPlace::Offset(offset),
                None => {
                    issues.push(format!(
                        "Where section {label} cuts could not be read or names a parameter the \
                         model no longer has, so it cuts through the middle of the bodies."
                    ));
                    SectionPlace::Centre
                }
            }
        }
        _ => {
            issues.push(format!(
                "Where section {label} cuts was not understood, so it cuts through the middle of \
                 the bodies."
            ));
            SectionPlace::Centre
        }
    }
}

fn restored_section(
    record: &SectionRecord,
    taken: &[SectionView],
    name: &str,
    exists: &dyn Fn(ParameterId) -> bool,
    issues: &mut Vec<String>,
) -> Option<SectionView> {
    let letter = view_letter(&record.letter, |letter| {
        taken.iter().any(|section| section.letter == letter)
    })?;
    let label = format!("{letter}\u{2013}{letter}");
    if record.letter != letter.to_string() {
        issues.push(format!(
            "A section view of the drawing “{name}” had no letter of its own, so it is {label}."
        ));
    }
    let parent = StandardView::from_key(&record.parent)
        .filter(|view| StandardView::SECTIONED.contains(view))
        .unwrap_or_else(|| {
            issues.push(format!(
                "The view section {label} is cut on was not understood, so it is the front view."
            ));
            StandardView::Front
        });
    let plane = SectionPlane::from_key(&record.plane)
        .filter(|plane| plane.is_edge_on_in(parent))
        .or_else(|| {
            let fallback = SectionPlane::edge_on_in(parent).next();
            if let Some(fallback) = fallback {
                issues.push(format!(
                    "The plane of section {label} was not understood or cannot be cut on its \
                     view, so it is the {}.",
                    fallback.name()
                ));
            }
            fallback
        })?;
    Some(SectionView {
        letter,
        parent,
        plane,
        place: restored_place(&record.place, &label, exists, issues),
        reversed: record.reversed,
    })
}

fn restored_sections(
    records: &[Lenient<SectionRecord>],
    name: &str,
    exists: &dyn Fn(ParameterId) -> bool,
    issues: &mut Vec<String>,
) -> Vec<SectionView> {
    let mut sections = Vec::new();
    for record in records {
        let Lenient::Read(record) = record else {
            issues.push(format!(
                "A section view of the drawing “{name}” could not be read, so it was left out."
            ));
            continue;
        };
        if sections.len() >= MAX_SECTION_VIEWS {
            issues.push(format!(
                "A drawing holds at most {MAX_SECTION_VIEWS} section views, so the rest of \
                 those of “{name}” were left out."
            ));
            break;
        }
        if let Some(section) = restored_section(record, &sections, name, exists, issues) {
            sections.push(section);
        }
    }
    sections
}

fn restored_scale_of_detail(
    record: &Lenient<DetailScaleRecord>,
    label: char,
    issues: &mut Vec<String>,
) -> DetailScale {
    let ratio = match record {
        Lenient::Read(DetailScaleRecord::Enlarged) => return DetailScale::Enlarged,
        Lenient::Read(DetailScaleRecord::Ratio { sheet, model }) => ScaleRatio::new(*sheet, *model),
        Lenient::Unreadable(_) => {
            issues.push(format!(
                "The scale of detail view {label} was not understood, so it is twice the scale \
                 of the sheet."
            ));
            return DetailScale::Enlarged;
        }
    };
    if ratio.is_usable() {
        return DetailScale::Ratio(ratio);
    }
    issues.push(format!(
        "The scale {} of detail view {label} cannot be used, so it is twice the scale of the \
         sheet.",
        ratio.label()
    ));
    DetailScale::Enlarged
}

fn restored_parent(
    record: &Lenient<ParentRecord>,
    sections: &[SectionView],
) -> Option<DetailParent> {
    match record {
        Lenient::Read(ParentRecord::Standard(key)) => {
            StandardView::from_key(key).map(DetailParent::Standard)
        }
        Lenient::Read(ParentRecord::Section(letter)) => {
            let mut characters = letter.chars();
            let letter = characters
                .next()
                .filter(|_| characters.next().is_none())
                .filter(|letter| sections.iter().any(|section| section.letter == *letter))?;
            Some(DetailParent::Section(letter))
        }
        Lenient::Unreadable(_) => None,
    }
}

fn restored_detail(
    record: &DetailRecord,
    sections: &[SectionView],
    taken: &[DetailView],
    name: &str,
    issues: &mut Vec<String>,
) -> Option<DetailView> {
    let Some(letter) = view_letter(&record.letter, |letter| {
        sections.iter().any(|section| section.letter == letter)
            || taken.iter().any(|detail| detail.letter == letter)
    }) else {
        issues.push(format!(
            "Every letter of the drawing “{name}” is taken by a section or detail view, so a \
             detail view was left out."
        ));
        return None;
    };
    if record.letter != letter.to_string() {
        issues.push(format!(
            "A detail view of the drawing “{name}” had no letter of its own, so it is {letter}."
        ));
    }
    let Some(parent) = restored_parent(&record.parent, sections) else {
        issues.push(format!(
            "The view detail {letter} enlarges was not understood or is not in the drawing “{name}”, \
             so the detail view was left out."
        ));
        return None;
    };
    let [x, y] = record.centre;
    let centre = Point2::new(x, y);
    let usable = centre.is_finite()
        && x.abs() <= MAX_DETAIL_EXTENT
        && y.abs() <= MAX_DETAIL_EXTENT
        && record.radius.is_finite()
        && (MIN_DETAIL_RADIUS..=MAX_DETAIL_EXTENT).contains(&record.radius);
    if !usable {
        issues.push(format!(
            "The circle of detail view {letter} in the drawing “{name}” cannot be used, so the \
             detail view was left out."
        ));
        return None;
    }
    Some(DetailView {
        letter,
        parent,
        centre,
        radius: record.radius,
        scale: restored_scale_of_detail(&record.scale, letter, issues),
    })
}

fn restored_details(
    records: &[Lenient<DetailRecord>],
    sections: &[SectionView],
    name: &str,
    issues: &mut Vec<String>,
) -> Vec<DetailView> {
    let mut details = Vec::new();
    for record in records {
        let Lenient::Read(record) = record else {
            issues.push(format!(
                "A detail view of the drawing “{name}” could not be read, so it was left out."
            ));
            continue;
        };
        if details.len() >= MAX_DETAIL_VIEWS {
            issues.push(format!(
                "A drawing holds at most {MAX_DETAIL_VIEWS} detail views, so the rest of those of \
                 “{name}” were left out."
            ));
            break;
        }
        if let Some(detail) = restored_detail(record, sections, &details, name, issues) {
            details.push(detail);
        }
    }
    details
}

pub(crate) fn restore_drawings(
    record: DrawingsRecord,
    exists: &dyn Fn(ParameterId) -> bool,
    issues: &mut Vec<String>,
) -> Drawings {
    let mut drawings = Drawings {
        drawings: Vec::new(),
        next_id: record.next_id,
    };
    for drawing in record.drawings {
        let Lenient::Read(drawing) = drawing else {
            issues.push("A drawing could not be read, so it was left out.".to_owned());
            continue;
        };
        if drawings.drawings.len() >= MAX_DRAWINGS {
            issues.push(format!(
                "A model keeps at most {MAX_DRAWINGS} drawings, so the rest were left out."
            ));
            break;
        }
        let id = DrawingId::from_raw(drawing.id);
        if drawings.get(id).is_some() {
            issues.push(format!(
                "Two drawings had the same number, so “{}” was left out.",
                drawing.name
            ));
            continue;
        }
        let name = fitting_name(&drawing.name, &drawings, issues);
        let sections = restored_sections(&drawing.sections, &name, exists, issues);
        let details = restored_details(&drawing.details, &sections, &name, issues);
        let restored = Drawing {
            id,
            size: understood(
                &drawing.size,
                SheetSize::from_key,
                "sheet size",
                &name,
                SheetSize::name,
                issues,
            ),
            orientation: understood(
                &drawing.orientation,
                SheetOrientation::from_key,
                "sheet orientation",
                &name,
                SheetOrientation::name,
                issues,
            ),
            projection: understood(
                &drawing.projection,
                Projection::from_key,
                "projection",
                &name,
                Projection::name,
                issues,
            ),
            scale: restored_scale(&drawing.scale, &name, issues),
            views: restored_views(&drawing.views, &name, issues),
            bodies: restored_bodies(&drawing.bodies, &name, issues),
            hidden_lines: drawing.hidden_lines,
            sections,
            details,
            name,
        };
        drawings.drawings.push(restored);
    }
    drawings.next_id = drawings.first_free_id();
    drawings
}
