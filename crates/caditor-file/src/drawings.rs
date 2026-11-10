use std::collections::BTreeSet;

use caditor_document::{
    Document, Drawing, DrawingId, DrawingScale, Drawings, FIRST_UNSTORABLE_ID, FeatureId,
    MAX_DRAWING_BODIES, MAX_DRAWING_NAME_CHARS, MAX_DRAWINGS, MAX_SECTION_OFFSET,
    MAX_SECTION_VIEWS, Projection, SECTION_LETTERS, ScaleRatio, SectionPlace, SectionPlane,
    SectionView, SheetOrientation, SheetSize, StandardView, drawing_name,
};
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
}

fn section_record(section: &SectionView) -> SectionRecord {
    SectionRecord {
        letter: section.letter.to_string(),
        parent: section.parent.key().to_owned(),
        plane: section.plane.key().to_owned(),
        place: Lenient::Read(match section.place {
            SectionPlace::Centre => PlaceRecord::Centre,
            SectionPlace::Offset(offset) => PlaceRecord::Offset(offset),
        }),
        reversed: section.reversed,
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

fn section_letter(record: &str, taken: &[SectionView]) -> Option<char> {
    let mut characters = record.chars();
    let letter = characters
        .next()
        .filter(|_| characters.next().is_none())
        .filter(|letter| SECTION_LETTERS.contains(letter))
        .filter(|letter| taken.iter().all(|section| section.letter != *letter));
    letter.or_else(|| {
        SECTION_LETTERS
            .into_iter()
            .find(|letter| taken.iter().all(|section| section.letter != *letter))
    })
}

fn restored_place(
    record: &Lenient<PlaceRecord>,
    label: &str,
    issues: &mut Vec<String>,
) -> SectionPlace {
    match record {
        Lenient::Read(PlaceRecord::Centre) => SectionPlace::Centre,
        Lenient::Read(PlaceRecord::Offset(offset))
            if offset.is_finite() && offset.abs() <= MAX_SECTION_OFFSET =>
        {
            SectionPlace::Offset(*offset)
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
    issues: &mut Vec<String>,
) -> Option<SectionView> {
    let letter = section_letter(&record.letter, taken)?;
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
        place: restored_place(&record.place, &label, issues),
        reversed: record.reversed,
    })
}

fn restored_sections(
    records: &[Lenient<SectionRecord>],
    name: &str,
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
        if let Some(section) = restored_section(record, &sections, name, issues) {
            sections.push(section);
        }
    }
    sections
}

pub(crate) fn restore_drawings(record: DrawingsRecord, issues: &mut Vec<String>) -> Drawings {
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
            sections: restored_sections(&drawing.sections, &name, issues),
            name,
        };
        drawings.drawings.push(restored);
    }
    drawings.next_id = drawings.first_free_id();
    drawings
}
