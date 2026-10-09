mod path;
mod shapes;
mod sizing;
mod style;
mod syntax;
mod text;
mod xml;

use std::{
    collections::{BTreeMap, BTreeSet},
    rc::Rc,
};

use caditor_geometry::Vector2;

pub(super) use crate::import::svg::text::looks_like_svg;
use crate::{
    import::{
        Drawing, ImportError, MAX_DRAWING_ELEMENTS, MAX_DRAWING_POINTS, MAX_EXPANDED_OBJECTS,
        MAX_READ_CURVES,
        dxf::{capitalized, counted, flatten::flatten, geometry::Shape, list, were},
        model::unpacked,
        svg::{
            path::Outline,
            shapes::{Axis, length, nested_viewport, outline_of},
            sizing::{Sizing, sizing},
            style::Properties,
            syntax::Matrix,
            text::{LATIN_1_NOTE, decoded, is_packed},
            xml::{Node, Tree, XmlError, parse},
        },
    },
    untrusted::UntrustedMap,
};

pub const SVG_EXTENSIONS: [&str; 2] = ["svg", "svgz"];

const MAX_NESTING: usize = 100;
const MAX_USE_DEPTH: usize = 16;
const MAX_LAYER_NAME_CHARS: usize = 100;
const MAX_NAMED_ELEMENTS: usize = 8;
const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";
const XLINK_NAMESPACE: &str = "http://www.w3.org/1999/xlink";
const INKSCAPE_NAMESPACE: &str = "http://www.inkscape.org/namespaces/inkscape";
const UNGROUPED: &str = "Ungrouped";
const CONDITIONS: [&str; 3] = ["requiredFeatures", "requiredExtensions", "systemLanguage"];
const NOT_DRAWN: [&str; 30] = [
    "defs",
    "symbol",
    "title",
    "desc",
    "metadata",
    "style",
    "script",
    "clipPath",
    "mask",
    "pattern",
    "marker",
    "linearGradient",
    "radialGradient",
    "meshgradient",
    "stop",
    "filter",
    "font",
    "font-face",
    "glyph",
    "missing-glyph",
    "animate",
    "animateMotion",
    "animateTransform",
    "animateColor",
    "set",
    "mpath",
    "view",
    "cursor",
    "color-profile",
    "solidcolor",
];

pub fn parse_svg(bytes: &[u8]) -> Result<Drawing, ImportError> {
    let unpacked_bytes;
    let bytes = if is_packed(bytes) {
        unpacked_bytes = unpacked(bytes)?;
        unpacked_bytes.as_slice()
    } else {
        bytes
    };
    if !looks_like_svg(bytes) {
        return Err(ImportError::NotSvg);
    }
    let (text, latin) = decoded(bytes);
    let mut notes = Vec::new();
    if latin {
        notes.push(LATIN_1_NOTE.to_owned());
    }
    let tree = parse(&text, MAX_DRAWING_ELEMENTS).map_err(|error| match error {
        XmlError::TooManyElements => ImportError::TooManyElements,
        XmlError::Damaged { line } => ImportError::DamagedAt(line),
    })?;
    read_tree(&tree, notes)
}

fn read_tree(tree: &Tree<'_>, mut notes: Vec<String>) -> Result<Drawing, ImportError> {
    let damaged_at = tree.damaged_at;
    let Some(root) = tree.root() else {
        return Err(ImportError::NotSvg);
    };
    let namespace = root.namespace();
    if root.name() != "svg" || namespace.is_some_and(|uri| uri != SVG_NAMESPACE) {
        return match damaged_at {
            Some(line) => Err(ImportError::DamagedAt(line)),
            None => Err(ImportError::NotSvg),
        };
    }
    let sizing = sizing(root);
    notes.extend(sizing.note.clone());
    let mut walker = Walker::new(tree, namespace);
    walker.walk(root, &sizing)?;
    let mut drawing = flatten(
        &walker.shapes,
        &walker.shape_layers,
        &walker.construction,
        notes,
    );
    drawing.layers = std::mem::take(&mut walker.layer_names);
    drawing.unit_scale = sizing.unit_scale();
    match (walker.report(drawing), damaged_at) {
        (Ok(mut drawing), Some(line)) => {
            drawing.notes.push(format!(
                "The drawing is damaged near line {line}, so only what comes before it was \
                 imported."
            ));
            Ok(drawing)
        }
        (Err(ImportError::Empty { .. }), Some(line)) => Err(ImportError::DamagedAt(line)),
        (reported, _) => reported,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Group,
    Viewport,
    Drawn,
    Use,
    LeftOut(&'static str, &'static str),
    NotDrawn,
    Unsupported,
}

impl Kind {
    fn of(name: &str) -> Self {
        match name {
            "g" | "a" | "switch" => Self::Group,
            "svg" => Self::Viewport,
            "path" | "rect" | "circle" | "ellipse" | "line" | "polyline" | "polygon" => Self::Drawn,
            "use" => Self::Use,
            "text" => Self::LeftOut("text", "texts"),
            "image" => Self::LeftOut("image", "images"),
            "foreignObject" => Self::LeftOut("embedded object", "embedded objects"),
            name if NOT_DRAWN.contains(&name) || name.starts_with("fe") => Self::NotDrawn,
            _ => Self::Unsupported,
        }
    }

    fn forms_layer(self) -> bool {
        matches!(self, Self::Group | Self::Viewport)
    }

    fn is_geometry(self) -> bool {
        matches!(self, Self::Group | Self::Viewport | Self::Drawn | Self::Use)
    }
}

#[derive(Debug, Clone)]
struct Context {
    matrix: Matrix,
    viewport: Vector2,
    layer: Rc<str>,
    dashed: bool,
    visible: bool,
    depth: usize,
}

#[derive(Debug, Default)]
struct Tally {
    left_out: BTreeMap<(&'static str, &'static str), usize>,
    unsupported: usize,
    unsupported_names: BTreeSet<String>,
    hidden: usize,
    clipped: usize,
    damaged: usize,
    unreadable_transforms: usize,
    too_deep: usize,
    missing: usize,
    unusable: usize,
}

struct Walker<'a, 't> {
    namespace: Option<&'a str>,
    ids: UntrustedMap<&'a str, Node<'a, 't>>,
    layer_parent: usize,
    properties: BTreeMap<usize, Properties>,
    outlines: BTreeMap<usize, Rc<Outline>>,
    referencing: Vec<usize>,
    shapes: Vec<Shape>,
    shape_layers: Vec<usize>,
    construction: BTreeSet<usize>,
    layer_names: Vec<String>,
    layer_indices: BTreeMap<String, usize>,
    unnamed_groups: usize,
    points: usize,
    expanded: usize,
    beyond_the_limit: usize,
    tally: Tally,
}

impl<'a, 't> Walker<'a, 't> {
    fn new(tree: &'a Tree<'t>, namespace: Option<&'a str>) -> Self {
        let mut ids = UntrustedMap::new();
        for node in tree.nodes() {
            if let Some(id) = node.attribute("id") {
                ids.entry(id).or_insert(node);
            }
        }
        let root = tree.root();
        let mut walker = Self {
            namespace,
            ids,
            layer_parent: 0,
            properties: BTreeMap::new(),
            outlines: BTreeMap::new(),
            referencing: Vec::new(),
            shapes: Vec::new(),
            shape_layers: Vec::new(),
            construction: BTreeSet::new(),
            layer_names: Vec::new(),
            layer_indices: BTreeMap::new(),
            unnamed_groups: 0,
            points: 0,
            expanded: 0,
            beyond_the_limit: 0,
            tally: Tally::default(),
        };
        if let Some(root) = root {
            walker.layer_parent = walker.find_layer_parent(root).id();
        }
        walker
    }

    fn is_svg(&self, node: Node<'_, '_>) -> bool {
        node.namespace() == self.namespace
    }

    fn kind(&self, node: Node<'_, '_>) -> Option<Kind> {
        self.is_svg(node).then(|| Kind::of(node.name()))
    }

    fn find_layer_parent(&self, root: Node<'a, 't>) -> Node<'a, 't> {
        let mut parent = root;
        for _ in 0..MAX_NESTING {
            let mut drawn = parent.children().filter(|child| {
                self.kind(*child).is_some_and(Kind::is_geometry) && !Properties::of(*child).hidden
            });
            match (drawn.next(), drawn.next()) {
                (Some(only), None) if self.kind(only) == Some(Kind::Group) => parent = only,
                _ => break,
            }
        }
        parent
    }

    fn walk(&mut self, root: Node<'a, 't>, sizing: &Sizing) -> Result<(), ImportError> {
        let properties = self.properties(root);
        let transform = self.transform(properties);
        let context = Context {
            matrix: transform.then(&Matrix::scale(sizing.millimetres.x, sizing.millimetres.y)),
            viewport: sizing.viewport,
            layer: Rc::from(UNGROUPED),
            dashed: properties.dashed.unwrap_or(false),
            visible: properties.visibility.unwrap_or(true),
            depth: 1,
        };
        self.children(root, &context)
    }

    fn properties(&mut self, node: Node<'_, '_>) -> Properties {
        *self
            .properties
            .entry(node.id())
            .or_insert_with(|| Properties::of(node))
    }

    fn transform(&mut self, properties: Properties) -> Matrix {
        properties.transform.unwrap_or_else(|| {
            self.tally.unreadable_transforms += 1;
            Matrix::IDENTITY
        })
    }

    fn charge(&mut self, count: usize) -> Result<(), ImportError> {
        if self.referencing.is_empty() {
            return Ok(());
        }
        self.expanded = self.expanded.saturating_add(count);
        if self.expanded > MAX_EXPANDED_OBJECTS {
            return Err(ImportError::TooManyCopies);
        }
        Ok(())
    }

    fn children(&mut self, node: Node<'a, 't>, context: &Context) -> Result<(), ImportError> {
        let is_switch = self.is_svg(node) && node.name() == "switch";
        if is_switch {
            let mut elements = node.children().filter(|child| self.is_svg(*child));
            let first = elements.clone().next();
            let chosen = elements
                .find(|child| {
                    CONDITIONS
                        .iter()
                        .all(|name| child.attribute(name).is_none())
                })
                .or(first);
            return match chosen {
                Some(child) => self.visit(child, context),
                None => Ok(()),
            };
        }
        let names_layers = node.id() == self.layer_parent;
        for child in node.children() {
            if names_layers {
                let context = Context {
                    layer: self.layer_name(child),
                    ..context.clone()
                };
                self.visit(child, &context)?;
            } else {
                self.visit(child, context)?;
            }
        }
        Ok(())
    }

    fn layer_name(&mut self, node: Node<'_, '_>) -> Rc<str> {
        if !self.kind(node).is_some_and(Kind::forms_layer) {
            return Rc::from(UNGROUPED);
        }
        let named = node
            .attribute_in(INKSCAPE_NAMESPACE, "label")
            .or_else(|| node.attribute("id"))
            .map(str::trim)
            .filter(|name| !name.is_empty());
        match named {
            Some(name) => {
                let name: String = name
                    .chars()
                    .map(|character| {
                        if character.is_control() {
                            ' '
                        } else {
                            character
                        }
                    })
                    .take(MAX_LAYER_NAME_CHARS)
                    .collect();
                Rc::from(name.as_str())
            }
            None => {
                self.unnamed_groups += 1;
                Rc::from(format!("Group {}", self.unnamed_groups).as_str())
            }
        }
    }

    fn visit(&mut self, node: Node<'a, 't>, context: &Context) -> Result<(), ImportError> {
        let Some(kind) = self.kind(node) else {
            return Ok(());
        };
        match kind {
            Kind::NotDrawn => return Ok(()),
            Kind::LeftOut(singular, plural) => {
                *self.tally.left_out.entry((singular, plural)).or_default() += 1;
                return Ok(());
            }
            Kind::Unsupported => {
                self.tally.unsupported += 1;
                if self.tally.unsupported_names.len() < MAX_NAMED_ELEMENTS {
                    let name: String = node.name().chars().take(40).collect();
                    self.tally.unsupported_names.insert(name);
                }
                return Ok(());
            }
            Kind::Group | Kind::Viewport | Kind::Drawn | Kind::Use => {}
        }
        if context.depth >= MAX_NESTING {
            self.tally.too_deep += 1;
            return Ok(());
        }
        self.charge(1)?;
        let properties = self.properties(node);
        if properties.hidden {
            self.tally.hidden += 1;
            return Ok(());
        }
        if properties.clipped {
            self.tally.clipped += 1;
        }
        let transform = self.transform(properties);
        let mut inner = Context {
            matrix: transform.then(&context.matrix),
            dashed: properties.dashed.unwrap_or(context.dashed),
            visible: properties.visibility.unwrap_or(context.visible),
            depth: context.depth + 1,
            ..context.clone()
        };
        match kind {
            Kind::Viewport => {
                let (matrix, viewport) = nested_viewport(node, context.viewport);
                inner.matrix = matrix.then(&inner.matrix);
                inner.viewport = viewport;
                self.children(node, &inner)
            }
            Kind::Drawn => self.drawn(node, &inner),
            Kind::Use => self.used(node, inner),
            _ => self.children(node, &inner),
        }
    }

    fn used(&mut self, node: Node<'a, 't>, mut context: Context) -> Result<(), ImportError> {
        let target = node
            .attribute("href")
            .or_else(|| node.attribute_in(XLINK_NAMESPACE, "href"))
            .and_then(|reference| reference.trim().strip_prefix('#'))
            .and_then(|id| self.ids.get(id).copied());
        let Some(target) = target else {
            self.tally.missing += 1;
            return Ok(());
        };
        let looping = self.referencing.contains(&target.id())
            || node
                .ancestors()
                .any(|ancestor| ancestor.id() == target.id());
        if looping || self.referencing.len() >= MAX_USE_DEPTH {
            self.tally.too_deep += 1;
            return Ok(());
        }
        let x = length(node, "x", Axis::Horizontal, context.viewport).unwrap_or(0.0);
        let y = length(node, "y", Axis::Vertical, context.viewport).unwrap_or(0.0);
        context.matrix = Matrix::translation(x, y).then(&context.matrix);
        self.referencing.push(target.id());
        let is_symbol = self.is_svg(target) && target.name() == "symbol";
        let result = if is_symbol {
            self.symbol(target, &context)
        } else {
            self.visit(target, &context)
        };
        self.referencing.pop();
        result
    }

    fn symbol(&mut self, node: Node<'a, 't>, context: &Context) -> Result<(), ImportError> {
        self.charge(1)?;
        let properties = self.properties(node);
        if properties.hidden {
            self.tally.hidden += 1;
            return Ok(());
        }
        let transform = self.transform(properties);
        let inner = Context {
            matrix: transform.then(&context.matrix),
            dashed: properties.dashed.unwrap_or(context.dashed),
            visible: properties.visibility.unwrap_or(context.visible),
            depth: context.depth + 1,
            ..context.clone()
        };
        self.children(node, &inner)
    }

    fn drawn(&mut self, node: Node<'a, 't>, context: &Context) -> Result<(), ImportError> {
        if !context.visible {
            self.tally.hidden += 1;
            return Ok(());
        }
        let outline = match self.outlines.get(&node.id()) {
            Some(outline) => Rc::clone(outline),
            None => {
                let outline = Rc::new(outline_of(node, context.viewport));
                self.outlines.insert(node.id(), Rc::clone(&outline));
                outline
            }
        };
        if outline.damaged {
            self.tally.damaged += 1;
        }
        self.charge(outline.shapes.len())?;
        let room = MAX_READ_CURVES.saturating_sub(self.shapes.len());
        let beyond = outline.shapes.len().saturating_sub(room);
        self.beyond_the_limit = self.beyond_the_limit.saturating_add(beyond);
        let affine = context.matrix.affine();
        for shape in outline.shapes.iter().take(room) {
            self.push(shape.transformed(&affine), context)?;
        }
        Ok(())
    }

    fn push(&mut self, shape: Shape, context: &Context) -> Result<(), ImportError> {
        if !is_finite(&shape) {
            self.tally.unusable += 1;
            return Ok(());
        }
        self.points = self.points.saturating_add(shape.size());
        if self.points > MAX_DRAWING_POINTS {
            return Err(ImportError::TooDetailed);
        }
        let layer = self.layer_index(&context.layer);
        if context.dashed {
            self.construction.insert(self.shapes.len());
        }
        self.shapes.push(shape);
        self.shape_layers.push(layer);
        Ok(())
    }

    fn layer_index(&mut self, name: &str) -> usize {
        let key = name.to_lowercase();
        if let Some(index) = self.layer_indices.get(&key) {
            return *index;
        }
        let index = self.layer_names.len();
        self.layer_names.push(name.to_owned());
        self.layer_indices.insert(key, index);
        index
    }

    fn report(self, mut drawing: Drawing) -> Result<Drawing, ImportError> {
        let earlier = drawing.notes.len();
        let tally = self.tally;
        let notes = &mut drawing.notes;
        if !tally.left_out.is_empty() {
            let kinds: Vec<String> = tally
                .left_out
                .iter()
                .map(|((singular, plural), count)| counted(*count, singular, plural))
                .collect();
            let total = tally
                .left_out
                .values()
                .fold(0, |sum: usize, count| sum.saturating_add(*count));
            notes.push(format!(
                "{} {} left out, because sketches hold only points, lines, arcs, circles and \
                 splines.",
                capitalized(&list(&kinds)),
                were(total)
            ));
        }
        if tally.unsupported > 0 {
            let names: Vec<String> = tally.unsupported_names.into_iter().collect();
            notes.push(format!(
                "{} that caditor does not read {} left out: {}.",
                capitalized(&counted(tally.unsupported, "element", "elements")),
                were(tally.unsupported),
                list(&names)
            ));
        }
        let mut count = |count: usize, singular: &str, plural: &str, rest: &str| {
            if count > 0 {
                notes.push(format!(
                    "{} {} {rest}",
                    capitalized(&counted(count, singular, plural)),
                    were(count)
                ));
            }
        };
        count(
            tally.hidden,
            "hidden element",
            "hidden elements",
            "left out.",
        );
        count(
            tally.clipped,
            "element with a clip path or mask",
            "elements with a clip path or mask",
            "imported whole, without the clipping.",
        );
        count(
            tally.damaged,
            "path or point list with damaged data",
            "paths or point lists with damaged data",
            "imported up to the damage.",
        );
        count(
            tally.unreadable_transforms,
            "transform that could not be read",
            "transforms that could not be read",
            "ignored.",
        );
        count(
            tally.too_deep,
            "element nested too deeply or reusing itself",
            "elements nested too deeply or reusing themselves",
            "left out.",
        );
        count(
            tally.missing,
            "reference to an element the drawing does not contain",
            "references to elements the drawing does not contain",
            "left out.",
        );
        count(
            tally.unusable,
            "shape with numbers too large to draw",
            "shapes with numbers too large to draw",
            "left out.",
        );
        if self.beyond_the_limit > 0 {
            drawing.notes.push(format!(
                "Only the first {MAX_READ_CURVES} curves were read, more than a sketch can hold; \
                 {} more {} left out. Split the drawing to import the rest.",
                self.beyond_the_limit,
                were(self.beyond_the_limit)
            ));
        }
        if drawing.curves.is_empty() {
            return Err(ImportError::Empty {
                left_out: drawing.notes.split_off(earlier),
            });
        }
        Ok(drawing)
    }
}

fn is_finite(shape: &Shape) -> bool {
    let numbers_finite = match shape {
        Shape::Conic { start, sweep, .. } => start.is_finite() && sweep.is_finite(),
        _ => true,
    };
    numbers_finite
        && shape
            .defining_points()
            .iter()
            .all(|point| point.is_finite())
}
