use std::collections::BTreeMap;

use crate::read::graph::{Entity, Graph};

const STYLED_KINDS: [&str; 2] = ["STYLED_ITEM", "OVER_RIDING_STYLED_ITEM"];
const LAYER_KIND: &str = "PRESENTATION_LAYER_ASSIGNMENT";
const SEARCH_DEPTH: usize = 8;
const NAMED_COLOURS: [(&str, [u8; 3]); 8] = [
    ("red", [255, 0, 0]),
    ("green", [0, 255, 0]),
    ("blue", [0, 0, 255]),
    ("yellow", [255, 255, 0]),
    ("magenta", [255, 0, 255]),
    ("cyan", [0, 255, 255]),
    ("black", [0, 0, 0]),
    ("white", [255, 255, 255]),
];

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Look {
    pub colour: Option<[u8; 3]>,
    pub layer: Option<String>,
}

pub(crate) struct Presentation {
    colours: BTreeMap<u64, [u8; 3]>,
    layers: BTreeMap<u64, String>,
}

impl Presentation {
    pub fn of(graph: &Graph<'_>) -> Self {
        let mut colours = BTreeMap::new();
        let mut styled_items = BTreeMap::new();
        for entity in graph.entities() {
            let Some(fields) = STYLED_KINDS.iter().find_map(|kind| entity.find(kind)) else {
                continue;
            };
            let (Ok(styles), Ok(item)) = (fields.references(1), fields.reference(2)) else {
                continue;
            };
            styled_items.insert(entity.id, item);
            let found = styles
                .iter()
                .find_map(|style| surface_colour(graph, *style, SEARCH_DEPTH));
            if let Some(colour) = found {
                if entity.kind() == "OVER_RIDING_STYLED_ITEM" {
                    colours.insert(item, colour);
                } else {
                    colours.entry(item).or_insert(colour);
                }
            }
        }
        let mut layers = BTreeMap::new();
        for entity in graph.entities() {
            let Some(fields) = entity.find(LAYER_KIND) else {
                continue;
            };
            let (Some(name), Ok(items)) = (fields.name(0), fields.references(2)) else {
                continue;
            };
            for item in items {
                let item = styled_items.get(&item).copied().unwrap_or(item);
                layers.entry(item).or_insert_with(|| name.clone());
            }
        }
        Self { colours, layers }
    }

    pub fn of_solid(&self, graph: &Graph<'_>, solid: &Entity<'_>) -> Look {
        let shells = shells(solid);
        let faces: Vec<u64> = shells
            .iter()
            .filter_map(|shell| graph.entity(*shell).ok()?.fields().ok()?.references(1).ok())
            .flatten()
            .collect();
        Look {
            colour: shared(&self.colours, solid.id, &shells, &faces),
            layer: shared(&self.layers, solid.id, &shells, &faces),
        }
    }
}

fn shells(solid: &Entity<'_>) -> Vec<u64> {
    let Ok(fields) = solid.fields() else {
        return Vec::new();
    };
    match fields.reference(1) {
        Ok(shell) => vec![shell],
        Err(_) => fields.references(1).unwrap_or_default(),
    }
}

fn shared<T: Clone + PartialEq>(
    assigned: &BTreeMap<u64, T>,
    solid: u64,
    shells: &[u64],
    faces: &[u64],
) -> Option<T> {
    if let Some(value) = assigned.get(&solid) {
        return Some(value.clone());
    }
    if let Some(value) = shells.iter().find_map(|shell| assigned.get(shell)) {
        return Some(value.clone());
    }
    let mut seen: Option<&T> = None;
    for face in faces {
        let value = assigned.get(face)?;
        match seen {
            Some(earlier) if earlier != value => return None,
            _ => seen = Some(value),
        }
    }
    seen.cloned()
}

fn surface_colour(graph: &Graph<'_>, id: u64, depth: usize) -> Option<[u8; 3]> {
    let entity = graph.entity(id).ok()?;
    match entity.kind() {
        "COLOUR_RGB" => {
            let fields = entity.fields().ok()?;
            let channel = |index: usize| {
                fields
                    .real(index)
                    .ok()
                    .filter(|value| value.is_finite())
                    .map(|value| (value.clamp(0.0, 1.0) * 255.0).round() as u8)
            };
            Some([channel(1)?, channel(2)?, channel(3)?])
        }
        "DRAUGHTING_PRE_DEFINED_COLOUR" => {
            let name = entity.fields().ok()?.name(0)?.to_ascii_lowercase();
            NAMED_COLOURS
                .iter()
                .find(|(known, _)| *known == name)
                .map(|(_, colour)| *colour)
        }
        "CURVE_STYLE" | "POINT_STYLE" | "TEXT_STYLE" => None,
        _ if depth > 0 => entity
            .fields()
            .ok()?
            .all_references()
            .into_iter()
            .find_map(|next| surface_colour(graph, next, depth - 1)),
        _ => None,
    }
}
