use crate::import::svg::{
    syntax::{Length, Matrix, transform_list},
    xml::Node,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Properties {
    pub transform: Option<Matrix>,
    pub hidden: bool,
    pub visibility: Option<bool>,
    pub dashed: Option<bool>,
    pub clipped: bool,
}

impl Properties {
    pub fn of(node: Node<'_, '_>) -> Self {
        let style = declarations(node.attribute("style").unwrap_or_default());
        let property = |name: &str| {
            style
                .iter()
                .rev()
                .find(|(key, _)| key.eq_ignore_ascii_case(name))
                .map(|(_, value)| *value)
                .or_else(|| node.attribute(name).map(str::trim))
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
            clipped,
        }
    }
}

fn declarations(style: &str) -> Vec<(&str, &str)> {
    style
        .split(';')
        .filter_map(|declaration| declaration.split_once(':'))
        .map(|(name, value)| {
            let value = value.trim();
            let value = value
                .strip_suffix("!important")
                .map_or(value, str::trim_end);
            (name.trim(), value)
        })
        .collect()
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
