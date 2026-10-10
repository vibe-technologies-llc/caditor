mod css;
mod font;
mod lettering;
mod overlap;
mod path;
mod shapes;
mod sizing;
mod style;
mod syntax;
mod text;
mod text_style;
mod xml;

use std::{
    collections::{BTreeMap, BTreeSet},
    rc::Rc,
};

use caditor_geometry::{Point2, Vector2};

pub(super) use crate::import::svg::text::looks_like_svg;
use crate::{
    import::{
        Drawing, ImportError, MAX_DRAWING_ELEMENTS, MAX_DRAWING_POINTS, MAX_EXPANDED_OBJECTS,
        MAX_READ_CURVES,
        dxf::{capitalized, counted, flatten::flatten, geometry::Shape, list, were},
        model::unpacked,
        svg::{
            css::{MAX_STYLE_RULES, StyleSheet, without_comments},
            font::Typeface,
            path::{Outline, Vertex},
            shapes::{Axis, fitted, length, nested_viewport, outline_of, symbol_viewport},
            sizing::{Sizing, sizing},
            style::{Inherited, Properties},
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
const MARKED: [&str; 4] = ["path", "line", "polyline", "polygon"];
const STRAIGHT_BACK: f64 = 1e-12;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextOutlines<'f> {
    InFont {
        upright: &'f [u8],
        italic: Option<&'f [u8]>,
    },
    LeftOut,
}

pub fn parse_svg(bytes: &[u8], text_outlines: TextOutlines<'_>) -> Result<Drawing, ImportError> {
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
    read_tree(&tree, notes, text_outlines)
}

fn read_tree(
    tree: &Tree<'_>,
    mut notes: Vec<String>,
    text_outlines: TextOutlines<'_>,
) -> Result<Drawing, ImportError> {
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
    let style_text = style_text(tree, namespace);
    let sheet = StyleSheet::parse(&style_text);
    let typeface = match text_outlines {
        TextOutlines::InFont { upright, italic } => Typeface::parse(upright, italic),
        TextOutlines::LeftOut => None,
    };
    let mut walker = Walker::new(tree, namespace, &sheet, typeface);
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

fn style_text(tree: &Tree<'_>, namespace: Option<&str>) -> String {
    let mut text = String::new();
    for node in tree.nodes() {
        let is_css = node
            .attribute("type")
            .map(str::trim)
            .is_none_or(|kind| kind.is_empty() || kind.eq_ignore_ascii_case("text/css"));
        if node.name() == "style" && node.namespace() == namespace && is_css {
            without_comments(node.text(), &mut text);
        }
    }
    text
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MarkerPosition {
    Start,
    Middle,
    End,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Group,
    Viewport,
    Drawn,
    Use,
    Text,
    LeftOut(&'static str, &'static str),
    NotDrawn,
    Unsupported,
}

impl Kind {
    fn of(name: &str, outlines_text: bool) -> Self {
        match name {
            "g" | "a" | "switch" => Self::Group,
            "svg" => Self::Viewport,
            "path" | "rect" | "circle" | "ellipse" | "line" | "polyline" | "polygon" => Self::Drawn,
            "use" => Self::Use,
            "text" if outlines_text => Self::Text,
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
        matches!(
            self,
            Self::Group | Self::Viewport | Self::Drawn | Self::Use | Self::Text
        )
    }
}

#[derive(Debug, Clone)]
struct Context<'a> {
    matrix: Matrix,
    viewport: Vector2,
    layer: Rc<str>,
    style: Inherited<'a>,
    depth: usize,
}

#[derive(Debug, Default)]
struct Tally {
    left_out: BTreeMap<(&'static str, &'static str), usize>,
    unsupported: usize,
    unsupported_names: BTreeSet<String>,
    hidden: usize,
    unpainted: usize,
    clipped: usize,
    damaged: usize,
    unreadable_transforms: usize,
    too_deep: usize,
    missing: usize,
    unusable: usize,
    substituted_families: BTreeSet<String>,
    italic: usize,
    missing_letters: usize,
    on_path: usize,
}

struct Walker<'a, 't> {
    namespace: Option<&'a str>,
    sheet: &'a StyleSheet<'a>,
    ids: UntrustedMap<&'a str, Node<'a, 't>>,
    layer_parent: usize,
    properties: BTreeMap<usize, Properties<'a>>,
    marker_styles: BTreeMap<usize, Option<Inherited<'a>>>,
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
    typeface: Option<Typeface<'a>>,
}

impl<'a, 't> Walker<'a, 't> {
    fn new(
        tree: &'a Tree<'t>,
        namespace: Option<&'a str>,
        sheet: &'a StyleSheet<'a>,
        typeface: Option<Typeface<'a>>,
    ) -> Self {
        let mut ids = UntrustedMap::new();
        for node in tree.nodes() {
            if let Some(id) = node.attribute("id") {
                ids.entry(id).or_insert(node);
            }
        }
        let root = tree.root();
        let mut walker = Self {
            namespace,
            sheet,
            ids,
            layer_parent: 0,
            properties: BTreeMap::new(),
            marker_styles: BTreeMap::new(),
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
            typeface,
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
        self.is_svg(node)
            .then(|| Kind::of(node.name(), self.typeface.is_some()))
    }

    fn find_layer_parent(&mut self, root: Node<'a, 't>) -> Node<'a, 't> {
        let mut parent = root;
        for _ in 0..MAX_NESTING {
            let children: Vec<Node<'a, 't>> = parent
                .children()
                .filter(|child| self.kind(*child).is_some_and(Kind::is_geometry))
                .collect();
            let mut drawn = children
                .into_iter()
                .filter(|child| !self.properties(*child).hidden);
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
            style: Inherited::default().under(&properties),
            depth: 1,
        };
        self.children(root, &context)
    }

    fn properties(&mut self, node: Node<'a, 't>) -> Properties<'a> {
        let sheet = self.sheet;
        *self
            .properties
            .entry(node.id())
            .or_insert_with(|| Properties::of(node, sheet))
    }

    fn transform(&mut self, properties: Properties<'a>) -> Matrix {
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

    fn children(&mut self, node: Node<'a, 't>, context: &Context<'a>) -> Result<(), ImportError> {
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

    fn visit(&mut self, node: Node<'a, 't>, context: &Context<'a>) -> Result<(), ImportError> {
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
            Kind::Group | Kind::Viewport | Kind::Drawn | Kind::Use | Kind::Text => {}
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
            style: context.style.under(&properties),
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
            Kind::Text => self.text(node, &inner),
            Kind::Use => self.used(node, inner),
            _ => self.children(node, &inner),
        }
    }

    fn used(&mut self, node: Node<'a, 't>, mut context: Context<'a>) -> Result<(), ImportError> {
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
            self.symbol(target, node, &context)
        } else {
            self.visit(target, &context)
        };
        self.referencing.pop();
        result
    }

    fn symbol(
        &mut self,
        node: Node<'a, 't>,
        used: Node<'a, 't>,
        context: &Context<'a>,
    ) -> Result<(), ImportError> {
        self.charge(1)?;
        let properties = self.properties(node);
        if properties.hidden {
            self.tally.hidden += 1;
            return Ok(());
        }
        let transform = self.transform(properties);
        let Some((fitted, viewport)) = symbol_viewport(used, node, context.viewport) else {
            return Ok(());
        };
        let inner = Context {
            matrix: transform.then(&fitted).then(&context.matrix),
            viewport,
            style: context.style.under(&properties),
            depth: context.depth + 1,
            ..context.clone()
        };
        self.children(node, &inner)
    }

    fn drawn(&mut self, node: Node<'a, 't>, context: &Context<'a>) -> Result<(), ImportError> {
        if !context.style.visible {
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
        if context.style.unpainted() {
            self.tally.unpainted += 1;
        } else {
            self.place(&outline.shapes, &context.matrix, context)?;
        }
        if MARKED.contains(&node.name()) && context.style.markers.any() {
            self.markers(&outline.vertices, context)?;
        }
        Ok(())
    }

    fn markers(&mut self, vertices: &[Vertex], context: &Context<'a>) -> Result<(), ImportError> {
        let last = vertices.len().saturating_sub(1);
        let markers = context.style.markers;
        for (index, vertex) in vertices.iter().enumerate() {
            let placed = [
                (index == 0, markers.start, MarkerPosition::Start),
                (
                    index != 0 && index != last,
                    markers.middle,
                    MarkerPosition::Middle,
                ),
                (index == last, markers.end, MarkerPosition::End),
            ];
            for (applies, id, position) in placed {
                if let (true, Some(id)) = (applies, id) {
                    self.marker(id, vertex, position, context)?;
                }
            }
        }
        Ok(())
    }

    fn marker(
        &mut self,
        id: &str,
        vertex: &Vertex,
        position: MarkerPosition,
        context: &Context<'a>,
    ) -> Result<(), ImportError> {
        let target = self
            .ids
            .get(id)
            .copied()
            .filter(|node| self.is_svg(*node) && node.name() == "marker");
        let Some(marker) = target else {
            self.tally.missing += 1;
            return Ok(());
        };
        let style = self.marker_style(marker);
        let Some(style) = style.filter(|_| {
            !self.referencing.contains(&marker.id()) && self.referencing.len() < MAX_USE_DEPTH
        }) else {
            self.tally.too_deep += 1;
            return Ok(());
        };
        let viewport = context.viewport;
        let width = length(marker, "markerWidth", Axis::Horizontal, viewport).unwrap_or(3.0);
        let height = length(marker, "markerHeight", Axis::Vertical, viewport).unwrap_or(3.0);
        if width <= 0.0 || height <= 0.0 {
            return Ok(());
        }
        let (fitted, inner_viewport) = fitted(marker, Vector2::new(width, height));
        let reference = fitted.apply(Point2::new(
            length(marker, "refX", Axis::Horizontal, inner_viewport).unwrap_or(0.0),
            length(marker, "refY", Axis::Vertical, inner_viewport).unwrap_or(0.0),
        ));
        let user_space = marker
            .attribute("markerUnits")
            .is_some_and(|units| units.trim() == "userSpaceOnUse");
        let scale = if user_space {
            1.0
        } else {
            context.style.stroke_width.map_or(1.0, |stroke_width| {
                stroke_width.pixels(Axis::Diagonal.reference(viewport))
            })
        };
        let angle = orientation(marker.attribute("orient"), vertex, position);
        let matrix = fitted
            .then(&Matrix::translation(-reference.x, -reference.y))
            .then(&Matrix::scale(scale, scale))
            .then(&Matrix::rotation(angle))
            .then(&Matrix::translation(vertex.at.x, vertex.at.y))
            .then(&context.matrix);
        self.referencing.push(marker.id());
        let inner = Context {
            matrix,
            viewport: inner_viewport,
            style,
            depth: context.depth + 1,
            ..context.clone()
        };
        let result = self.charge(1).and_then(|()| self.children(marker, &inner));
        self.referencing.pop();
        result
    }

    fn marker_style(&mut self, marker: Node<'a, 't>) -> Option<Inherited<'a>> {
        if let Some(style) = self.marker_styles.get(&marker.id()) {
            return *style;
        }
        let lineage: Vec<Node<'a, 't>> = marker.ancestors().take(MAX_NESTING + 1).collect();
        let style = if lineage.len() > MAX_NESTING {
            None
        } else {
            let mut style = Inherited::default();
            for ancestor in lineage.into_iter().rev() {
                let properties = self.properties(ancestor);
                style = style.under(&properties);
            }
            style.markers = Default::default();
            Some(style)
        };
        self.marker_styles.insert(marker.id(), style);
        style
    }

    fn place(
        &mut self,
        shapes: &[Shape],
        matrix: &Matrix,
        context: &Context<'a>,
    ) -> Result<(), ImportError> {
        self.charge(shapes.len())?;
        let room = MAX_READ_CURVES.saturating_sub(self.shapes.len());
        let beyond = shapes.len().saturating_sub(room);
        self.beyond_the_limit = self.beyond_the_limit.saturating_add(beyond);
        let affine = matrix.affine();
        for shape in shapes.iter().take(room) {
            self.push(shape.transformed(&affine), context)?;
        }
        Ok(())
    }

    fn push(&mut self, shape: Shape, context: &Context<'a>) -> Result<(), ImportError> {
        if !is_finite(&shape) {
            self.tally.unusable += 1;
            return Ok(());
        }
        self.points = self.points.saturating_add(shape.size());
        if self.points > MAX_DRAWING_POINTS {
            return Err(ImportError::TooDetailed);
        }
        let layer = self.layer_index(&context.layer);
        if context.style.dashed {
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
            tally.unpainted,
            "element drawn with neither stroke nor fill",
            "elements drawn with neither stroke nor fill",
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
        count(
            tally.on_path,
            "text laid along a path",
            "texts laid along a path",
            "left out, since caditor sets text only along a line.",
        );
        count(
            tally.missing_letters,
            "character Inter has no letter for",
            "characters Inter has no letter for",
            "left out.",
        );
        count(
            tally.italic,
            "text in italic",
            "texts in italic",
            "drawn upright, since caditor carries only Inter's upright letters.",
        );
        if !tally.substituted_families.is_empty() {
            let families: Vec<String> = tally.substituted_families.into_iter().collect();
            notes.push(format!(
                "Text set in {} was drawn in Inter, the font caditor carries, so its letters \
                 differ in shape and width from the drawing's.",
                list(&families)
            ));
        }
        let sheet = self.sheet;
        if sheet.unread_selectors > 0 {
            drawing.notes.push(format!(
                "{} in the drawing's style sheet {} ignored, because caditor reads only selectors \
                 of an element name, class or id.",
                capitalized(&counted(sheet.unread_selectors, "rule", "rules")),
                were(sheet.unread_selectors)
            ));
        }
        if sheet.beyond_the_limit > 0 {
            drawing.notes.push(format!(
                "Only the first {MAX_STYLE_RULES} rules of the drawing's style sheet were read; {} \
                 more {} ignored.",
                sheet.beyond_the_limit,
                were(sheet.beyond_the_limit)
            ));
        }
        if sheet.exhausted() {
            drawing.notes.push(
                "The drawing's style sheet is too large to match against every element, so later \
                 elements were styled by their own attributes only."
                    .to_owned(),
            );
        }
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

fn orientation(orient: Option<&str>, vertex: &Vertex, position: MarkerPosition) -> f64 {
    let orient = orient.unwrap_or_default().trim();
    let reversed = match orient {
        "auto" => false,
        "auto-start-reverse" => position == MarkerPosition::Start,
        angle => return degrees(angle).unwrap_or(0.0),
    };
    let heading = |direction: Vector2| direction.y.atan2(direction.x).to_degrees();
    let angle = match (vertex.incoming, vertex.outgoing) {
        (Some(incoming), Some(outgoing)) => {
            let between = incoming.normalize_or_zero() + outgoing.normalize_or_zero();
            if between.length_squared() > STRAIGHT_BACK {
                heading(between)
            } else {
                heading(incoming)
            }
        }
        (Some(direction), None) | (None, Some(direction)) => heading(direction),
        (None, None) => 0.0,
    };
    if reversed { angle + 180.0 } else { angle }
}

fn degrees(text: &str) -> Option<f64> {
    let units = [
        ("deg", 1.0),
        ("grad", 0.9),
        ("rad", 180.0 / std::f64::consts::PI),
        ("turn", 360.0),
        ("", 1.0),
    ];
    units.into_iter().find_map(|(unit, factor)| {
        let number = text.strip_suffix(unit)?.trim_end();
        let value: f64 = number.parse().ok()?;
        value.is_finite().then_some(value * factor)
    })
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
