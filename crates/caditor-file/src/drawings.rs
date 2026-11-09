use std::collections::BTreeSet;

use caditor_document::{
    Document, Drawing, DrawingId, DrawingScale, Drawings, FIRST_UNSTORABLE_ID, FeatureId,
    MAX_DRAWING_BODIES, MAX_DRAWING_NAME_CHARS, MAX_DRAWINGS, Projection, ScaleRatio,
    SheetOrientation, SheetSize, StandardView, drawing_name,
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
            name,
        };
        drawings.drawings.push(restored);
    }
    drawings.next_id = drawings.first_free_id();
    drawings
}
