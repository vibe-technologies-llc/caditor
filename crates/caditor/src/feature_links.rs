use caditor_document::{Document, Feature, FeatureId};
use egui::{Color32, CornerRadius, Id, Pos2, Rect, Stroke, Ui, pos2, vec2};

use crate::{
    appearance::{self, BORDER_WIDTH, FOCUS_WIDTH, SPACE_S, SPACE_XS},
    feature_tree,
    tree_row::LINK_GUTTER,
};

pub const MARK_RADIUS: f32 = SPACE_S - BORDER_WIDTH;
const RAIL_WIDTH: f32 = BORDER_WIDTH;
const TICK_WIDTH: f32 = FOCUS_WIDTH;
pub const RING_WIDTH: f32 = FOCUS_WIDTH;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Links {
    pub uses: Vec<FeatureId>,
    pub used_by: Vec<FeatureId>,
}

impl Links {
    pub fn of(document: &Document, feature: &Feature) -> Self {
        let used = document.dependencies_of(&feature.kind);
        let uses = document
            .features()
            .map(Feature::id)
            .filter(|other| used.contains(other))
            .collect();
        let used_by = document
            .features()
            .filter(|other| {
                document
                    .dependencies_of(&other.kind)
                    .contains(&feature.id())
            })
            .map(Feature::id)
            .collect();
        Self { uses, used_by }
    }

    pub fn description(&self, document: &Document, feature: &Feature) -> String {
        let names = |ids: &[FeatureId]| -> Vec<String> {
            ids.iter()
                .filter_map(|id| document.feature(*id))
                .map(|related| related.name.clone())
                .collect()
        };
        let uses = match names(&self.uses).as_slice() {
            [] => format!("{} uses no other feature.", feature.name),
            named => format!("Uses {}.", feature_tree::in_words(named)),
        };
        let used_by = match names(&self.used_by).as_slice() {
            [] => format!("No feature uses {}.", feature.name),
            named => format!("Used by {}.", feature_tree::in_words(named)),
        };
        format!("{uses} {used_by}")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LinksBasis {
    feature: FeatureId,
    revision: u64,
}

#[derive(Debug, Clone, Default)]
pub struct LinksCache {
    kept: Option<(LinksBasis, Links)>,
    computed: usize,
}

impl LinksCache {
    pub fn links(&mut self, document: &Document, revision: u64, feature: &Feature) -> &Links {
        let basis = LinksBasis {
            feature: feature.id(),
            revision,
        };
        let kept = self
            .kept
            .take()
            .filter(|(kept, _)| *kept == basis)
            .unwrap_or_else(|| {
                self.computed = self.computed.saturating_add(1);
                (basis, Links::of(document, feature))
            });
        &self.kept.insert(kept).1
    }

    #[cfg(test)]
    pub fn computed(&self) -> usize {
        self.computed
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    Focus,
    Uses,
    UsedBy,
}

pub fn marks(focus: FeatureId, links: &Links) -> Vec<(FeatureId, Mark)> {
    std::iter::once((focus, Mark::Focus))
        .chain(links.uses.iter().map(|id| (*id, Mark::Uses)))
        .chain(links.used_by.iter().map(|id| (*id, Mark::UsedBy)))
        .collect()
}

pub fn paint(ui: &Ui, placed: &[(FeatureId, Rect)], marks: &[(FeatureId, Mark)]) {
    let tokens = appearance::tokens(ui);
    let shown: Vec<(Rect, Mark)> = marks
        .iter()
        .filter_map(|(id, mark)| {
            placed
                .iter()
                .find(|(placed, _)| placed == id)
                .map(|(_, rect)| (*rect, *mark))
        })
        .collect();
    if shown.len() < 2 {
        return;
    }
    let x = shown
        .iter()
        .map(|(rect, _)| rect.left())
        .fold(f32::INFINITY, f32::min)
        + SPACE_XS
        + LINK_GUTTER / 2.0;
    let top = shown
        .iter()
        .map(|(rect, _)| rect.center().y)
        .fold(f32::INFINITY, f32::min);
    let bottom = shown
        .iter()
        .map(|(rect, _)| rect.center().y)
        .fold(f32::NEG_INFINITY, f32::max);
    let painter = ui.painter();
    painter.line_segment(
        [pos2(x, top), pos2(x, bottom)],
        Stroke::new(RAIL_WIDTH, tokens.text_muted),
    );
    for (rect, mark) in shown {
        let centre = pos2(x, rect.center().y);
        match mark {
            Mark::Focus => focus_tick(ui, centre, rect, tokens.text),
            Mark::Uses => {
                painter.circle_filled(centre, MARK_RADIUS, tokens.text_muted);
            }
            Mark::UsedBy => {
                painter.circle_stroke(
                    centre,
                    MARK_RADIUS - RING_WIDTH / 2.0,
                    Stroke::new(RING_WIDTH, tokens.text_muted),
                );
            }
        }
    }
}

fn focus_tick(ui: &Ui, centre: Pos2, row: Rect, colour: Color32) {
    let tick = Rect::from_min_max(
        pos2(centre.x - MARK_RADIUS, centre.y - TICK_WIDTH / 2.0),
        pos2(row.left() + LINK_GUTTER, centre.y + TICK_WIDTH / 2.0),
    );
    let bar = Rect::from_center_size(centre, vec2(TICK_WIDTH, MARK_RADIUS * 2.0));
    let painter = ui.painter();
    painter.rect_filled(tick, CornerRadius::ZERO, colour);
    painter.rect_filled(bar, CornerRadius::ZERO, colour);
}

pub fn row_id(feature: FeatureId) -> Id {
    Id::unique(("feature-row", feature))
}
