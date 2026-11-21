use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use caditor_geometry::larger;
use caditor_kernel::{Interrupted, check_interrupt};

use crate::read::{
    finish::{Finish, appearance_finish},
    geometry::Work,
    graph::{Entity, Graph},
    topology::{SolidShells, shell_faces},
};

const LAYER_KIND: &str = "PRESENTATION_LAYER_ASSIGNMENT";
const MAX_STYLE_DEPTH: usize = 8;
const OPAQUE_PERCENT: u8 = 100;
const CHANNEL_SLACK: f64 = 1e-6;
const BYTE_CHANNELS: f64 = 255.0;
const MOST_NAMED_PROBLEMS: usize = 5;
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
const CURVE_STYLES: [&str; 11] = [
    "CURVE_STYLE",
    "POINT_STYLE",
    "TEXT_STYLE",
    "TEXT_STYLE_WITH_BOX_CHARACTERISTICS",
    "SYMBOL_STYLE",
    "SURFACE_STYLE_BOUNDARY",
    "SURFACE_STYLE_PARAMETER_LINE",
    "SURFACE_STYLE_SILHOUETTE",
    "SURFACE_STYLE_SEGMENTATION_CURVE",
    "SURFACE_STYLE_CONTROL_GRID",
    "PRE_DEFINED_PRESENTATION_STYLE",
];
const SEE_THROUGH_APPEARANCES: [(&str, u8); 7] = [
    ("clear", 25),
    ("transparent", 25),
    ("glass", 25),
    ("translucent", 50),
    ("smoked", 50),
    ("frosted", 50),
    ("tinted", 50),
];
const OPAQUE_APPEARANCES: [&str; 3] = ["opaque", "coat", "clearcoat"];
const REFLECTANCES: [&str; 3] = [
    "SURFACE_STYLE_REFLECTANCE_AMBIENT",
    "SURFACE_STYLE_REFLECTANCE_AMBIENT_DIFFUSE",
    "SURFACE_STYLE_REFLECTANCE_AMBIENT_DIFFUSE_SPECULAR",
];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Look {
    pub colour: Option<[u8; 3]>,
    pub opacity: Option<u8>,
    pub finish: Option<Finish>,
    pub layer: Option<String>,
    pub faces: Vec<FaceLook>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FaceLook {
    pub face: usize,
    pub colour: Option<[u8; 3]>,
    pub opacity: Option<u8>,
    pub finish: Option<Finish>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Stated {
    colour: Option<[u8; 3]>,
    opacity: Option<u8>,
    finish: Option<Finish>,
    plain: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Effective {
    colour: Option<[u8; 3]>,
    opacity: u8,
    finish: Option<Finish>,
}

impl Stated {
    fn states_anything(self) -> bool {
        self.colour.is_some() || self.opacity.is_some() || self.plain
    }

    fn under(self, newer: Self) -> Self {
        let painted = if newer.colour.is_some() { newer } else { self };
        Self {
            colour: painted.colour,
            opacity: newer.opacity.or(self.opacity),
            finish: painted.finish,
            plain: newer.plain || self.plain,
        }
    }

    fn opacity_or_opaque(self) -> u8 {
        self.opacity.unwrap_or(OPAQUE_PERCENT)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Unread {
    Colour,
    NamedColour,
    Transparency,
    Hatching,
    Style,
    Missing,
    UnmatchedCopy,
    TooIntricate,
}

impl Unread {
    fn describe(self, entity: u64) -> String {
        match self {
            Self::Colour => format!("#{entity}, a colour caditor cannot read"),
            Self::NamedColour => {
                format!("#{entity}, a colour named in a way caditor does not know")
            }
            Self::Transparency => {
                format!("#{entity}, a transparency that is not between 0 and 1")
            }
            Self::Hatching => format!("#{entity}, a hatched or tiled fill"),
            Self::Style => format!("#{entity}, a kind of style caditor does not know"),
            Self::Missing => format!("#{entity}, a style the file does not hold"),
            Self::UnmatchedCopy => {
                format!("#{entity}, a style for one copy in the assembly that matches none of them")
            }
            Self::TooIntricate => format!("#{entity}, a style too intricate to read"),
        }
    }
}

struct Instanced {
    styled: u64,
    context: Arc<[u64]>,
    by_context: Option<u64>,
    stated: Stated,
}

impl Instanced {
    fn elements(&self) -> impl Iterator<Item = &u64> {
        self.context.iter().chain(&self.by_context)
    }

    fn length(&self) -> usize {
        self.context.len() + usize::from(self.by_context.is_some())
    }
}

pub(crate) struct Presentation {
    styles: BTreeMap<u64, Stated>,
    overrides: BTreeMap<u64, Stated>,
    instanced: BTreeMap<u64, Vec<Instanced>>,
    layers: BTreeMap<u64, Arc<str>>,
    unread: BTreeMap<u64, Unread>,
    troubles: BTreeMap<u64, BTreeSet<u64>>,
}

#[derive(Debug, Default)]
pub(crate) struct Consulted {
    items: BTreeSet<u64>,
    matched: BTreeSet<u64>,
}

pub(crate) struct SolidParts {
    id: u64,
    shells: Vec<u64>,
    face_shells: BTreeMap<u64, Vec<u64>>,
    faces: Vec<u64>,
}

impl SolidParts {
    pub fn of(graph: &Graph<'_>, solid: u64) -> Self {
        let mut parts = Self {
            id: solid,
            shells: Vec::new(),
            face_shells: BTreeMap::new(),
            faces: Vec::new(),
        };
        let Ok(shells) = SolidShells::of(graph, solid) else {
            return parts;
        };

        for shell in shells.all() {
            let mut ids = vec![shell];

            if let Ok(inner) = graph
                .entity(shell)
                .and_then(|entity| entity.record("ORIENTED_CLOSED_SHELL"))
                .and_then(|fields| fields.reference(2))
            {
                ids.push(inner);
            }
            for face in shell_faces(*graph, shell).unwrap_or_default() {
                parts.faces.push(face);
                parts
                    .face_shells
                    .entry(face)
                    .or_default()
                    .extend(ids.iter().copied());
            }
            parts.shells.extend(ids);
        }
        parts
    }
}

struct Styling {
    styles: Vec<u64>,
    item: u64,
    overriding: bool,
    context: Option<Arc<[u64]>>,
}

fn once_each(ids: Vec<u64>) -> Vec<u64> {
    let mut seen = BTreeSet::new();
    ids.into_iter().filter(|id| seen.insert(*id)).collect()
}

fn styling(entity: &Entity<'_>) -> Option<Styling> {
    if let Ok(fields) = entity.fields() {
        let kind = entity.kind();
        let overriding = match kind {
            "STYLED_ITEM" => false,
            "OVER_RIDING_STYLED_ITEM" | "CONTEXT_DEPENDENT_OVER_RIDING_STYLED_ITEM" => true,
            _ => return None,
        };
        let context = (kind == "CONTEXT_DEPENDENT_OVER_RIDING_STYLED_ITEM")
            .then(|| Arc::from(once_each(fields.references(4).unwrap_or_default())));

        return Some(Styling {
            styles: once_each(fields.references(1).ok()?),
            item: fields.reference(2).ok()?,
            overriding,
            context,
        });
    }

    let fields = entity.find("STYLED_ITEM")?;
    Some(Styling {
        styles: once_each(fields.references(0).ok()?),
        item: fields.reference(1).ok()?,
        overriding: entity.is("OVER_RIDING_STYLED_ITEM"),
        context: entity
            .find("CONTEXT_DEPENDENT_OVER_RIDING_STYLED_ITEM")
            .map(|fields| Arc::from(once_each(fields.references(0).unwrap_or_default()))),
    })
}

fn inner_item(graph: &Graph<'_>, item: u64) -> u64 {
    graph
        .entity(item)
        .ok()
        .filter(|entity| entity.kind() == "ORIENTED_FACE")
        .and_then(|entity| entity.record("ORIENTED_FACE").ok()?.reference(2).ok())
        .unwrap_or(item)
}

impl Presentation {
    pub fn of(graph: &Graph<'_>, work: &Work) -> Result<Self, Interrupted> {
        let mut presentation = Self {
            styles: BTreeMap::new(),
            overrides: BTreeMap::new(),
            instanced: BTreeMap::new(),
            layers: BTreeMap::new(),
            unread: BTreeMap::new(),
            troubles: BTreeMap::new(),
        };
        let mut styled_items = BTreeMap::new();
        let mut resolved: BTreeMap<u64, Resolved> = BTreeMap::new();

        for entity in graph.entities() {
            check_interrupt()?;

            let Some(styling) = styling(&entity) else {
                continue;
            };

            styled_items.insert(entity.id, styling.item);

            let item = inner_item(graph, styling.item);
            let mut problems = BTreeMap::new();
            let mut general = Sides::default();
            let mut by_context: Vec<(u64, Sides)> = Vec::new();

            for style in &styling.styles {
                if !resolved.contains_key(style) {
                    let mut reading = Reading::new(graph, work);
                    let mut sides = Sides::default();

                    reading.style(*style, Side::Both, &mut sides, MAX_STYLE_DEPTH);
                    if reading.cancelled {
                        return Err(Interrupted);
                    }

                    let context = reading.context_of(*style);

                    resolved.insert(
                        *style,
                        Resolved {
                            context,
                            sides,
                            problems: reading.problems,
                        },
                    );
                }

                let Some(read) = resolved.get(style) else {
                    continue;
                };

                problems.extend(read.problems.iter().map(|(id, unread)| (*id, *unread)));
                match read.context {
                    Some(context) => by_context.push((context, read.sides)),
                    None => general.merge(read.sides),
                }
            }
            if !problems.is_empty() {
                presentation
                    .troubles
                    .entry(item)
                    .or_default()
                    .extend(problems.keys().copied());
                presentation.unread.extend(problems);
            }

            let shared: Arc<[u64]> = styling.context.clone().unwrap_or_else(|| Arc::from([]));
            let stated = general.stated();

            if stated.states_anything() {
                match &styling.context {
                    Some(context) => {
                        presentation.instance(item, entity.id, Arc::clone(context), None, stated);
                    }
                    None if styling.overriding => {
                        let earlier = presentation.overrides.entry(item).or_default();

                        *earlier = earlier.under(stated);
                    }
                    None => {
                        let earlier = presentation.styles.entry(item).or_default();

                        *earlier = stated.under(*earlier);
                    }
                }
            }
            for (context, sides) in by_context {
                let stated = sides.stated();
                if stated.states_anything() {
                    presentation.instance(
                        item,
                        entity.id,
                        Arc::clone(&shared),
                        Some(context),
                        stated,
                    );
                }
            }
        }
        for entity in graph.entities() {
            let Some(fields) = entity.find(LAYER_KIND) else {
                continue;
            };
            let (Some(name), Ok(items)) = (fields.name(0), fields.references(2)) else {
                continue;
            };
            let name: Arc<str> = Arc::from(name);
            for item in items {
                let item = styled_items.get(&item).copied().unwrap_or(item);

                presentation
                    .layers
                    .entry(inner_item(graph, item))
                    .or_insert_with(|| Arc::clone(&name));
            }
        }
        Ok(presentation)
    }

    fn instance(
        &mut self,
        item: u64,
        styled: u64,
        context: Arc<[u64]>,
        by_context: Option<u64>,
        stated: Stated,
    ) {
        self.instanced.entry(item).or_default().push(Instanced {
            styled,
            context,
            by_context,
            stated,
        });
    }

    pub fn placement_key(&self, path: &[u64]) -> Vec<u64> {
        if !self.instanced.is_empty() {
            return path.to_vec();
        }
        path.iter()
            .copied()
            .filter(|item| {
                self.styles.contains_key(item)
                    || self.overrides.contains_key(item)
                    || self.troubles.contains_key(item)
            })
            .collect()
    }

    fn stated(&self, item: u64, path: &[u64], consulted: &mut Consulted) -> Stated {
        if self.troubles.contains_key(&item) {
            consulted.items.insert(item);
        }

        let mut stated = self.styles.get(&item).copied().unwrap_or_default();

        if let Some(overriding) = self.overrides.get(&item) {
            stated = stated.under(*overriding);
        }

        let Some(instances) = self.instanced.get(&item) else {
            return stated;
        };

        consulted.items.insert(item);

        let matching = instances
            .iter()
            .filter(|instance| instance.elements().all(|element| path.contains(element)))
            .max_by_key(|instance| instance.length());
        match matching {
            Some(instance) => {
                consulted.matched.insert(instance.styled);
                stated.under(instance.stated)
            }
            None => stated,
        }
    }

    pub fn look(
        &self,
        parts: &SolidParts,
        faces: &[Option<u64>],
        path: &[u64],
        consulted: &mut Consulted,
    ) -> Look {
        let mut above = vec![self.stated(parts.id, path, consulted)];

        for shell in &parts.shells {
            above.push(self.stated(*shell, path, consulted));
        }
        for container in path {
            above.push(self.stated(*container, path, consulted));
        }

        let above: Vec<Stated> = above
            .into_iter()
            .filter(|stated| stated.states_anything())
            .collect();
        let mut face_looks = Vec::with_capacity(faces.len());

        for entity in faces {
            let Some(entity) = entity else {
                face_looks.push(None);
                continue;
            };
            let mut own = vec![self.stated(*entity, path, consulted)];

            for shell in parts.face_shells.get(entity).into_iter().flatten() {
                own.push(self.stated(*shell, path, consulted));
            }

            let levels: Vec<Stated> = own
                .into_iter()
                .filter(|stated| stated.states_anything())
                .chain(above.iter().copied())
                .collect();

            face_looks.push(Some(effective(&levels)));
        }

        let body = if above.is_empty() {
            let present: Vec<Effective> = face_looks.iter().flatten().copied().collect();
            let colour = most_shared(present.iter().map(|look| look.colour)).flatten();
            Effective {
                colour,
                opacity: most_shared(present.iter().map(|look| look.opacity))
                    .unwrap_or(OPAQUE_PERCENT),
                finish: present
                    .iter()
                    .find(|look| look.colour == colour)
                    .and_then(|look| look.finish),
            }
        } else {
            effective(&above)
        };
        let faces = face_looks
            .iter()
            .enumerate()
            .filter_map(|(face, look)| {
                let look = (*look)?;
                let colour = look.colour.filter(|colour| Some(*colour) != body.colour);
                let opacity = (look.opacity != body.opacity).then_some(look.opacity);
                let finish = look.finish.filter(|finish| Some(*finish) != body.finish);
                (colour.is_some() || opacity.is_some() || finish.is_some()).then_some(FaceLook {
                    face,
                    colour,
                    opacity,
                    finish,
                })
            })
            .collect();
        let opacity = body.opacity;
        Look {
            colour: body.colour,
            opacity: (opacity < OPAQUE_PERCENT).then_some(opacity),
            finish: body.finish,
            layer: self.layer(parts),
            faces,
        }
    }

    fn layer(&self, parts: &SolidParts) -> Option<String> {
        if let Some(layer) = self.layers.get(&parts.id) {
            return Some(layer.to_string());
        }
        if let Some(layer) = parts.shells.iter().find_map(|shell| self.layers.get(shell)) {
            return Some(layer.to_string());
        }

        let mut seen: Option<&Arc<str>> = None;

        for face in &parts.faces {
            let layer = self.layers.get(face)?;
            match seen {
                Some(earlier) if earlier != layer => return None,
                _ => seen = Some(layer),
            }
        }
        seen.map(ToString::to_string)
    }

    pub fn note(&self, consulted: &Consulted) -> Option<String> {
        let mut problems: BTreeMap<u64, Unread> = BTreeMap::new();

        for item in &consulted.items {
            for entity in self.troubles.get(item).into_iter().flatten() {
                if let Some(unread) = self.unread.get(entity) {
                    problems.insert(*entity, *unread);
                }
            }
            for instance in self.instanced.get(item).into_iter().flatten() {
                if !consulted.matched.contains(&instance.styled) {
                    problems.insert(instance.styled, Unread::UnmatchedCopy);
                }
            }
        }

        let count = problems.len();
        let named: Vec<String> = problems
            .iter()
            .take(MOST_NAMED_PROBLEMS)
            .map(|(entity, unread)| unread.describe(*entity))
            .collect();

        let listed = match named.as_slice() {
            [] => return None,
            [only] => only.clone(),
            [rest @ .., last] if count == named.len() => format!("{} and {last}", rest.join("; ")),
            _ => format!("{} and {} more", named.join("; "), count - named.len()),
        };
        Some(format!(
            "Some styling in the file could not be understood, so what it styles keeps the look \
             it has without it: {listed}."
        ))
    }
}

fn effective(levels: &[Stated]) -> Effective {
    let painted = levels.iter().find(|stated| stated.colour.is_some());
    let opacity = levels
        .iter()
        .find(|stated| stated.states_anything())
        .map_or(OPAQUE_PERCENT, |stated| stated.opacity_or_opaque());
    Effective {
        colour: painted.and_then(|stated| stated.colour),
        opacity,
        finish: painted.and_then(|stated| stated.finish),
    }
}

fn most_shared<T: PartialEq + Copy>(values: impl IntoIterator<Item = T>) -> Option<T> {
    let mut counted: Vec<(T, usize)> = Vec::new();

    for value in values {
        match counted.iter_mut().find(|(seen, _)| *seen == value) {
            Some((_, count)) => *count += 1,
            None => counted.push((value, 1)),
        }
    }

    let most = counted.iter().map(|(_, count)| *count).max()?;
    counted
        .into_iter()
        .find(|(_, count)| *count == most)
        .map(|(value, _)| value)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Side {
    Outside,
    Inside,
    Both,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Paint {
    colour: [u8; 3],
    appearance_opacity: Option<u8>,
    appearance_finish: Option<Finish>,
}

#[derive(Clone, Copy, Debug, Default)]
struct Found {
    fill: Option<Paint>,
    rendering: Option<Paint>,
    opacity: Option<u8>,
    plain: bool,
}

impl Found {
    fn stated(self) -> Stated {
        let paint = self.fill.or(self.rendering);
        Stated {
            colour: paint.map(|paint| paint.colour),
            opacity: self
                .opacity
                .or(paint.and_then(|paint| paint.appearance_opacity)),
            finish: paint.and_then(|paint| paint.appearance_finish),
            plain: self.plain,
        }
    }
}

fn appearance_opacity(name: &str) -> Option<u8> {
    let words: Vec<String> = name
        .split(|letter: char| !letter.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect();

    if words
        .iter()
        .any(|word| OPAQUE_APPEARANCES.contains(&word.as_str()))
    {
        return None;
    }
    SEE_THROUGH_APPEARANCES
        .iter()
        .filter(|(word, _)| words.iter().any(|named| named == word))
        .map(|(_, opacity)| *opacity)
        .max()
}

#[derive(Clone, Copy, Debug, Default)]
struct Sides {
    outside: Found,
    inside: Found,
}

impl Found {
    fn merge(&mut self, later: Self) {
        self.fill = self.fill.or(later.fill);
        self.rendering = self.rendering.or(later.rendering);
        self.opacity = self.opacity.or(later.opacity);
        self.plain |= later.plain;
    }
}

impl Sides {
    fn merge(&mut self, later: Self) {
        self.outside.merge(later.outside);
        self.inside.merge(later.inside);
    }

    fn each(&mut self, side: Side, mut change: impl FnMut(&mut Found)) {
        if side != Side::Inside {
            change(&mut self.outside);
        }
        if side != Side::Outside {
            change(&mut self.inside);
        }
    }

    fn stated(self) -> Stated {
        let outside = self.outside.stated();
        if outside.states_anything() {
            outside
        } else {
            self.inside.stated()
        }
    }
}

struct Resolved {
    context: Option<u64>,
    sides: Sides,
    problems: BTreeMap<u64, Unread>,
}

struct Reading<'g, 'a> {
    graph: &'g Graph<'a>,
    work: &'g Work,
    problems: BTreeMap<u64, Unread>,
    visited: BTreeMap<(u64, Side), usize>,
    cancelled: bool,
}

impl<'g, 'a> Reading<'g, 'a> {
    fn new(graph: &'g Graph<'a>, work: &'g Work) -> Self {
        Self {
            graph,
            work,
            problems: BTreeMap::new(),
            visited: BTreeMap::new(),
            cancelled: false,
        }
    }

    fn context_of(&self, style: u64) -> Option<u64> {
        let entity = self.graph.entity(style).ok()?;

        if entity.kind() != "PRESENTATION_STYLE_BY_CONTEXT" {
            return None;
        }

        let fields = entity.fields().ok()?;
        fields.reference(1).ok()
    }

    fn style(&mut self, id: u64, side: Side, found: &mut Sides, depth: usize) {
        let Some(depth) = depth.checked_sub(1) else {
            return;
        };

        if self.cancelled || check_interrupt().is_err() {
            self.cancelled = true;
            return;
        }
        if self
            .visited
            .get(&(id, side))
            .is_some_and(|seen| *seen >= depth)
        {
            return;
        }
        self.visited.insert((id, side), depth);
        if self.work.charge(id).is_err() {
            self.problems.insert(id, Unread::TooIntricate);
            return;
        }

        let Ok(entity) = self.graph.entity(id) else {
            self.problems.insert(id, Unread::Missing);
            return;
        };
        let kind = entity.kind();

        if CURVE_STYLES.contains(&kind) || REFLECTANCES.contains(&kind) {
            return;
        }

        let Ok(fields) = entity.fields() else {
            self.problems.insert(id, Unread::Style);
            return;
        };
        match kind {
            "PRESENTATION_STYLE_ASSIGNMENT" | "PRESENTATION_STYLE_BY_CONTEXT" => {
                let Ok(styles) = fields.list(0) else {
                    self.problems.insert(id, Unread::Style);
                    return;
                };
                for style in styles.iter() {
                    if let Some(reference) = style.reference() {
                        self.style(reference, side, found, depth);
                    } else if style.typed().is_some_and(|(name, _)| name == "NULL_STYLE")
                        || style.enumeration() == Some("NULL")
                    {
                        found.each(side, |found| found.plain = true);
                    }
                }
            }
            "SURFACE_STYLE_USAGE" => {
                let side = match fields.get(0).ok().and_then(|value| value.enumeration()) {
                    Some("POSITIVE") => Side::Outside,
                    Some("NEGATIVE") => Side::Inside,
                    _ => side,
                };
                match fields.reference(1) {
                    Ok(style) => self.style(style, side, found, depth),
                    Err(_) => {
                        self.problems.insert(id, Unread::Style);
                    }
                }
            }
            "SURFACE_SIDE_STYLE" => match fields.references(1) {
                Ok(elements) => {
                    for element in elements {
                        self.style(element, side, found, depth);
                    }
                }
                Err(_) => {
                    self.problems.insert(id, Unread::Style);
                }
            },
            "SURFACE_STYLE_FILL_AREA" => match fields.reference(0) {
                Ok(fill) => self.style(fill, side, found, depth),
                Err(_) => {
                    self.problems.insert(id, Unread::Style);
                }
            },
            "FILL_AREA_STYLE" => match fields.references(1) {
                Ok(fills) => {
                    for fill in fills {
                        self.style(fill, side, found, depth);
                    }
                }
                Err(_) => {
                    self.problems.insert(id, Unread::Style);
                }
            },
            "FILL_AREA_STYLE_COLOUR" => {
                if let Some(paint) = fields
                    .reference(1)
                    .ok()
                    .and_then(|colour| self.colour(colour))
                {
                    found.each(side, |found| {
                        found.fill.get_or_insert(paint);
                    });
                }
            }
            "FILL_AREA_STYLE_HATCHING"
            | "FILL_AREA_STYLE_TILES"
            | "EXTERNALLY_DEFINED_HATCH_STYLE"
            | "EXTERNALLY_DEFINED_TILE_STYLE" => {
                self.problems.insert(id, Unread::Hatching);
            }
            "SURFACE_STYLE_RENDERING" | "SURFACE_STYLE_RENDERING_WITH_PROPERTIES" => {
                if let Some(paint) = fields
                    .optional_reference(1)
                    .and_then(|colour| self.colour(colour))
                {
                    found.each(side, |found| {
                        found.rendering.get_or_insert(paint);
                    });
                }
                if kind == "SURFACE_STYLE_RENDERING_WITH_PROPERTIES" {
                    for property in fields.references(2).unwrap_or_default() {
                        self.style(property, side, found, depth);
                    }
                }
            }
            "SURFACE_STYLE_TRANSPARENT" => match fields.real(0) {
                Ok(transparency)
                    if (-CHANNEL_SLACK..=1.0 + CHANNEL_SLACK).contains(&transparency) =>
                {
                    let opacity = ((1.0 - transparency.clamp(0.0, 1.0)) * 100.0).round() as u8;

                    found.each(side, |found| {
                        found.opacity.get_or_insert(opacity);
                    });
                }
                _ => {
                    self.problems.insert(id, Unread::Transparency);
                }
            },
            _ => {
                self.problems.insert(id, Unread::Style);
            }
        }
    }

    fn colour(&mut self, id: u64) -> Option<Paint> {
        let Ok(entity) = self.graph.entity(id) else {
            self.problems.insert(id, Unread::Missing);
            return None;
        };
        let read = match entity.kind() {
            "COLOUR_RGB" => {
                let (channels, name) = match entity.fields() {
                    Ok(fields) => (
                        [1, 2, 3].map(|index| fields.real(index).ok()),
                        fields.name(0),
                    ),
                    Err(_) => (
                        entity.find("COLOUR_RGB").map_or([None; 3], |fields| {
                            [0, 1, 2].map(|index| fields.real(index).ok())
                        }),
                        None,
                    ),
                };
                rgb(channels)
                    .map(|colour| Paint {
                        colour,
                        appearance_opacity: name.as_deref().and_then(appearance_opacity),
                        appearance_finish: name.as_deref().and_then(appearance_finish),
                    })
                    .ok_or(Unread::Colour)
            }
            "DRAUGHTING_PRE_DEFINED_COLOUR" | "PRE_DEFINED_COLOUR" => {
                let name = entity
                    .fields()
                    .ok()
                    .and_then(|fields| fields.name(0))
                    .map(|name| name.to_ascii_lowercase());
                name.and_then(|name| {
                    NAMED_COLOURS
                        .iter()
                        .find(|(known, _)| *known == name)
                        .map(|(_, colour)| Paint {
                            colour: *colour,
                            appearance_opacity: None,
                            appearance_finish: None,
                        })
                })
                .ok_or(Unread::NamedColour)
            }
            _ => Err(Unread::Colour),
        };
        match read {
            Ok(paint) => Some(paint),
            Err(unread) => {
                self.problems.insert(id, unread);
                None
            }
        }
    }
}

fn rgb(channels: [Option<f64>; 3]) -> Option<[u8; 3]> {
    let [Some(red), Some(green), Some(blue)] = channels else {
        return None;
    };

    let values = [red, green, blue];
    let low = values.iter().copied().fold(f64::INFINITY, f64::min);
    let high = values.iter().copied().fold(f64::NEG_INFINITY, larger);

    if low < -CHANNEL_SLACK || high > BYTE_CHANNELS + CHANNEL_SLACK {
        return None;
    }

    let scale = if high > 1.0 + CHANNEL_SLACK {
        1.0
    } else {
        BYTE_CHANNELS
    };
    Some(values.map(|value| (value * scale).round().clamp(0.0, BYTE_CHANNELS) as u8))
}

#[cfg(test)]
mod tests {
    use caditor_kernel::interruptible;

    use super::*;
    use crate::part21::parse;

    const ASSIGNMENTS: u64 = 8;
    const REPEATS: usize = 50;
    const STYLED_ITEMS: u64 = 3;

    fn crafted() -> String {
        let mut data = vec!["#1=CARTESIAN_POINT('',(0.,0.,0.));".to_owned()];

        for level in 0..ASSIGNMENTS {
            let next = vec![format!("#{}", 11 + level); REPEATS].join(",");

            data.push(format!(
                "#{}=PRESENTATION_STYLE_ASSIGNMENT(({next}));",
                10 + level
            ));
        }
        for styled in 0..STYLED_ITEMS {
            data.push(format!("#{}=STYLED_ITEM('',(#10),#1);", 100 + styled));
        }
        format!(
            "ISO-10303-21;\nHEADER;\nENDSEC;\nDATA;\n{}\nENDSEC;\nEND-ISO-10303-21;\n",
            data.join("\n")
        )
    }

    #[test]
    fn styles_that_list_each_other_many_times_are_read_once_each_within_a_small_budget() {
        let text = crafted();
        let exchange = parse(&text).unwrap();
        let graph = Graph::new(&exchange);
        let one_visit_each = Work::new(ASSIGNMENTS as usize);
        let starved = Work::new(ASSIGNMENTS as usize - 1);

        let read = Presentation::of(&graph, &one_visit_each).unwrap();
        let cut = Presentation::of(&graph, &starved).unwrap();

        assert!(read.unread.is_empty(), "{:?}", read.unread);
        assert_eq!(cut.unread.len(), 1);
        assert!(
            cut.unread
                .values()
                .all(|unread| *unread == Unread::TooIntricate)
        );
    }

    #[test]
    fn an_interrupt_stops_the_reading_of_styles() {
        let text = crafted();
        let exchange = parse(&text).unwrap();
        let graph = Graph::new(&exchange);
        let work = Work::new(usize::MAX);

        let stopped = interruptible(Arc::new(|| true), || Presentation::of(&graph, &work));

        assert!(stopped.is_err());
    }
}
