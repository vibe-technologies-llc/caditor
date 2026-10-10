use crate::import::svg::{
    css::{Declaration, StyleSheet, inline_declarations},
    syntax::{Length, Matrix, transform_list},
    text_style::{TextProperties, TextStyle},
    xml::{Node, XML_NAMESPACE},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MarkerChoice<'a> {
    Nothing,
    Element(&'a str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) struct Markers<'a> {
    pub start: Option<&'a str>,
    pub middle: Option<&'a str>,
    pub end: Option<&'a str>,
}

impl Markers<'_> {
    pub fn any(&self) -> bool {
        self.start.is_some() || self.middle.is_some() || self.end.is_some()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Properties<'a> {
    pub transform: Option<Matrix>,
    pub hidden: bool,
    pub visibility: Option<bool>,
    pub dashed: Option<bool>,
    pub stroke: Option<bool>,
    pub fill: Option<bool>,
    pub stroke_width: Option<Length>,
    pub marker_start: Option<MarkerChoice<'a>>,
    pub marker_middle: Option<MarkerChoice<'a>>,
    pub marker_end: Option<MarkerChoice<'a>>,
    pub clipped: bool,
    pub text: TextProperties<'a>,
}

impl<'a> Properties<'a> {
    pub fn of(node: Node<'a, '_>, sheet: &StyleSheet<'a>) -> Self {
        let mut found = sheet.declarations(node);
        found.extend(inline_declarations(
            node.attribute("style").unwrap_or_default(),
        ));
        let declared = |names: &[&str]| {
            found
                .iter()
                .filter(|(_, declaration)| {
                    names
                        .iter()
                        .any(|name| declaration.name.eq_ignore_ascii_case(name))
                })
                .max_by_key(|(precedence, _)| *precedence)
                .map(|(_, declaration)| *declaration)
        };
        let cascaded = |names: &[&str]| declared(names).map(|Declaration { value, .. }| value);
        let property =
            |name: &str| cascaded(&[name]).or_else(|| node.attribute(name).map(str::trim));
        let marker = |name: &str| {
            cascaded(&[name, "marker"])
                .or_else(|| node.attribute(name).map(str::trim))
                .and_then(marker_choice)
        };
        let transform = match node.attribute("transform") {
            Some(text) => transform_list(text),
            None => Some(Matrix::IDENTITY),
        };
        let visibility = match property("visibility") {
            Some("hidden" | "collapse") => Some(false),
            Some("visible") => Some(true),
            _ => None,
        };
        let clipped = ["clip-path", "mask"]
            .into_iter()
            .any(|name| property(name).is_some_and(|value| !value.is_empty() && value != "none"));
        Self {
            transform,
            hidden: property("display") == Some("none"),
            visibility,
            dashed: property("stroke-dasharray").and_then(is_dashed),
            stroke: property("stroke").and_then(is_painted),
            fill: property("fill").and_then(is_painted),
            stroke_width: property("stroke-width")
                .and_then(Length::parse)
                .filter(|width| width.value >= 0.0 && width.value.is_finite()),
            marker_start: marker("marker-start"),
            marker_middle: marker("marker-mid"),
            marker_end: marker("marker-end"),
            clipped,
            text: TextProperties::of(
                property,
                |name| declared(&[name, "font"]),
                node.attribute_in(XML_NAMESPACE, "space"),
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Inherited<'a> {
    pub dashed: bool,
    pub visible: bool,
    pub stroke: Option<bool>,
    pub fill: Option<bool>,
    pub stroke_width: Option<Length>,
    pub markers: Markers<'a>,
    pub text: TextStyle<'a>,
}

impl Default for Inherited<'_> {
    fn default() -> Self {
        Self {
            dashed: false,
            visible: true,
            stroke: None,
            fill: None,
            stroke_width: None,
            markers: Markers::default(),
            text: TextStyle::default(),
        }
    }
}

impl<'a> Inherited<'a> {
    pub fn under(self, properties: &Properties<'a>) -> Self {
        let chosen = |choice: Option<MarkerChoice<'a>>, inherited: Option<&'a str>| match choice {
            Some(MarkerChoice::Nothing) => None,
            Some(MarkerChoice::Element(id)) => Some(id),
            None => inherited,
        };
        Self {
            dashed: properties.dashed.unwrap_or(self.dashed),
            visible: properties.visibility.unwrap_or(self.visible),
            stroke: properties.stroke.or(self.stroke),
            fill: properties.fill.or(self.fill),
            stroke_width: properties.stroke_width.or(self.stroke_width),
            markers: Markers {
                start: chosen(properties.marker_start, self.markers.start),
                middle: chosen(properties.marker_middle, self.markers.middle),
                end: chosen(properties.marker_end, self.markers.end),
            },
            text: self.text.under(&properties.text),
        }
    }

    pub fn unpainted(&self) -> bool {
        self.stroke == Some(false) && self.fill == Some(false)
    }
}

fn is_painted(value: &str) -> Option<bool> {
    match value {
        "" | "inherit" => None,
        value => Some(!value.eq_ignore_ascii_case("none")),
    }
}

fn marker_choice(value: &str) -> Option<MarkerChoice<'_>> {
    if value.eq_ignore_ascii_case("none") {
        return Some(MarkerChoice::Nothing);
    }
    let inner = value.strip_prefix("url(")?.strip_suffix(')')?.trim();
    let unquoted = ['"', '\'']
        .into_iter()
        .find_map(|quote| inner.strip_prefix(quote)?.strip_suffix(quote))
        .unwrap_or(inner);
    let id = unquoted.trim().strip_prefix('#')?;
    Some(MarkerChoice::Element(id))
}

fn is_dashed(value: &str) -> Option<bool> {
    if value == "none" {
        return Some(false);
    }
    let lengths: Option<Vec<f64>> = value
        .split(|character: char| character == ',' || character.is_ascii_whitespace())
        .filter(|part| !part.is_empty())
        .map(|part| Length::parse(part).map(|length| length.value))
        .collect();
    let lengths = lengths?;
    if lengths.is_empty() || lengths.iter().any(|length| *length < 0.0) {
        return None;
    }
    Some(lengths.iter().any(|length| *length > 0.0))
}
