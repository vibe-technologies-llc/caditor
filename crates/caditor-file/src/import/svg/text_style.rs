use crate::import::svg::{
    css::Declaration,
    syntax::{Length, Scanner, Unit},
};

pub(super) const MEDIUM_FONT_SIZE: f64 = 16.0;
const NORMAL_WEIGHT: f64 = 400.0;
const BOLD_WEIGHT: f64 = 700.0;
const LIGHTEST_WEIGHT: f64 = 1.0;
const HEAVIEST_WEIGHT: f64 = 1000.0;
const SIZE_STEP: f64 = 1.2;
const EX_PER_EM: f64 = 0.5;
const SUBSCRIPT_SHIFT_EMS: f64 = -0.2;
const SUPERSCRIPT_SHIFT_EMS: f64 = 0.4;
const SHORTHAND: &str = "font";
const KEYWORD_SIZES: [(&str, f64); 8] = [
    ("xx-small", 9.0),
    ("x-small", 10.0),
    ("small", 13.0),
    ("medium", MEDIUM_FONT_SIZE),
    ("large", 18.0),
    ("x-large", 24.0),
    ("xx-large", 32.0),
    ("xxx-large", 48.0),
];
const SYSTEM_FONTS: [&str; 6] = [
    "caption",
    "icon",
    "menu",
    "message-box",
    "small-caption",
    "status-bar",
];
const SHORTHAND_KEYWORDS: [&str; 10] = [
    "normal",
    "small-caps",
    "ultra-condensed",
    "extra-condensed",
    "condensed",
    "semi-condensed",
    "semi-expanded",
    "expanded",
    "extra-expanded",
    "ultra-expanded",
];

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum FontSize {
    Pixels(f64),
    OfParent(f64),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum FontWeight {
    Absolute(f64),
    Bolder,
    Lighter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum Anchor {
    #[default]
    Start,
    Middle,
    End,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Spacing {
    Pixels(f64),
    Ems(f64),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum BaselineShift {
    Pixels(f64),
    Ems(f64),
    OfParentSize(f64),
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(super) struct TextProperties<'a> {
    pub size: Option<FontSize>,
    pub family: Option<&'a str>,
    pub weight: Option<FontWeight>,
    pub italic: Option<bool>,
    pub anchor: Option<Anchor>,
    pub letter_spacing: Option<Spacing>,
    pub word_spacing: Option<Spacing>,
    pub preserve_space: Option<bool>,
    pub baseline_shift: Option<BaselineShift>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Shorthand<'a> {
    size: FontSize,
    family: &'a str,
    weight: FontWeight,
    italic: bool,
}

enum Longhand<'a> {
    Value(Option<&'a str>),
    Shorthand(Option<Shorthand<'a>>),
}

impl<'a> TextProperties<'a> {
    pub fn of(
        property: impl Fn(&str) -> Option<&'a str>,
        with_shorthand: impl Fn(&str) -> Option<Declaration<'a>>,
        space: Option<&str>,
    ) -> Self {
        let longhand = |name: &str| match with_shorthand(name) {
            Some(declaration) if declaration.name.eq_ignore_ascii_case(SHORTHAND) => {
                Longhand::Shorthand(shorthand(declaration.value))
            }
            Some(declaration) => Longhand::Value(Some(declaration.value)),
            None => Longhand::Value(property(name)),
        };
        Self {
            size: chosen(longhand("font-size"), font_size, |font| font.size),
            family: chosen(longhand("font-family"), family, |font| font.family),
            weight: chosen(longhand("font-weight"), font_weight, |font| font.weight),
            italic: chosen(longhand("font-style"), italic, |font| font.italic),
            anchor: property("text-anchor").and_then(anchor),
            letter_spacing: property("letter-spacing").and_then(spacing),
            word_spacing: property("word-spacing").and_then(spacing),
            baseline_shift: property("baseline-shift").and_then(baseline_shift),
            preserve_space: space.and_then(|space| match space.trim() {
                "preserve" => Some(true),
                "default" => Some(false),
                _ => None,
            }),
        }
    }
}

fn chosen<'a, T>(
    longhand: Longhand<'a>,
    read: fn(&'a str) -> Option<T>,
    part: fn(Shorthand<'a>) -> T,
) -> Option<T> {
    match longhand {
        Longhand::Value(value) => value.and_then(read),
        Longhand::Shorthand(font) => font.map(part),
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct TextStyle<'a> {
    pub size: f64,
    pub family: Option<&'a str>,
    pub weight: f64,
    pub italic: bool,
    pub anchor: Anchor,
    pub letter_spacing: f64,
    pub word_spacing: f64,
    pub preserve_space: bool,
    pub rise: f64,
}

impl Default for TextStyle<'_> {
    fn default() -> Self {
        Self {
            size: MEDIUM_FONT_SIZE,
            family: None,
            weight: NORMAL_WEIGHT,
            italic: false,
            anchor: Anchor::Start,
            letter_spacing: 0.0,
            word_spacing: 0.0,
            preserve_space: false,
            rise: 0.0,
        }
    }
}

impl<'a> TextStyle<'a> {
    pub fn under(self, properties: &TextProperties<'a>) -> Self {
        let size = match properties.size {
            Some(FontSize::Pixels(size)) => size,
            Some(FontSize::OfParent(factor)) => self.size * factor,
            None => self.size,
        };
        let weight = match properties.weight {
            Some(FontWeight::Absolute(weight)) => weight,
            Some(FontWeight::Bolder) => bolder(self.weight),
            Some(FontWeight::Lighter) => lighter(self.weight),
            None => self.weight,
        };
        let spaced = |spacing: Option<Spacing>, inherited: f64| match spacing {
            Some(Spacing::Pixels(pixels)) => pixels,
            Some(Spacing::Ems(ems)) => ems * size,
            None => inherited,
        };
        let shift = match properties.baseline_shift {
            Some(BaselineShift::Pixels(pixels)) => pixels,
            Some(BaselineShift::Ems(ems)) => ems * size,
            Some(BaselineShift::OfParentSize(share)) => share * self.size,
            None => 0.0,
        };
        Self {
            size,
            family: properties.family.or(self.family),
            weight,
            italic: properties.italic.unwrap_or(self.italic),
            anchor: properties.anchor.unwrap_or(self.anchor),
            letter_spacing: spaced(properties.letter_spacing, self.letter_spacing),
            word_spacing: spaced(properties.word_spacing, self.word_spacing),
            preserve_space: properties.preserve_space.unwrap_or(self.preserve_space),
            rise: self.rise + shift,
        }
    }
}

fn bolder(weight: f64) -> f64 {
    match weight {
        weight if weight < 350.0 => NORMAL_WEIGHT,
        weight if weight < 550.0 => BOLD_WEIGHT,
        weight if weight < 750.0 => 900.0,
        weight => weight.max(900.0),
    }
}

fn lighter(weight: f64) -> f64 {
    match weight {
        weight if weight < 100.0 => weight,
        weight if weight < 550.0 => 100.0,
        weight if weight < 750.0 => NORMAL_WEIGHT,
        _ => BOLD_WEIGHT,
    }
}

fn font_size(value: &str) -> Option<FontSize> {
    let value = value.trim();
    if let Some((_, size)) = KEYWORD_SIZES
        .iter()
        .find(|(keyword, _)| value.eq_ignore_ascii_case(keyword))
    {
        return Some(FontSize::Pixels(*size));
    }
    if value.eq_ignore_ascii_case("larger") {
        return Some(FontSize::OfParent(SIZE_STEP));
    }
    if value.eq_ignore_ascii_case("smaller") {
        return Some(FontSize::OfParent(1.0 / SIZE_STEP));
    }
    let size = relative(value, FontSize::OfParent, FontSize::Pixels).or_else(|| {
        let length = Length::parse(value)?;
        Some(match length.unit {
            Unit::Percent => FontSize::OfParent(length.value / 100.0),
            _ => FontSize::Pixels(length.pixels(0.0)),
        })
    })?;
    let usable = match size {
        FontSize::Pixels(amount) | FontSize::OfParent(amount) => {
            amount.is_finite() && amount >= 0.0
        }
    };
    usable.then_some(size)
}

fn relative<T>(value: &str, ems: impl Fn(f64) -> T, pixels: impl Fn(f64) -> T) -> Option<T> {
    let mut scanner = Scanner::new(value);
    let number = scanner.number()?;
    let unit = std::str::from_utf8(scanner.rest())
        .ok()?
        .to_ascii_lowercase();
    match unit.as_str() {
        "em" => Some(ems(number)),
        "ex" => Some(ems(number * EX_PER_EM)),
        "rem" => Some(pixels(number * MEDIUM_FONT_SIZE)),
        _ => None,
    }
}

fn family(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty() && !value.eq_ignore_ascii_case("inherit")).then_some(value)
}

fn font_weight(value: &str) -> Option<FontWeight> {
    let value = value.trim();
    match value.to_ascii_lowercase().as_str() {
        "normal" => Some(FontWeight::Absolute(NORMAL_WEIGHT)),
        "bold" => Some(FontWeight::Absolute(BOLD_WEIGHT)),
        "bolder" => Some(FontWeight::Bolder),
        "lighter" => Some(FontWeight::Lighter),
        _ => value
            .parse::<f64>()
            .ok()
            .filter(|weight| (LIGHTEST_WEIGHT..=HEAVIEST_WEIGHT).contains(weight))
            .map(FontWeight::Absolute),
    }
}

fn italic(value: &str) -> Option<bool> {
    let value = value.trim().to_ascii_lowercase();
    match value.split_whitespace().next() {
        Some("normal") => Some(false),
        Some("italic" | "oblique") => Some(true),
        _ => None,
    }
}

fn anchor(value: &str) -> Option<Anchor> {
    match value.trim() {
        "start" => Some(Anchor::Start),
        "middle" => Some(Anchor::Middle),
        "end" => Some(Anchor::End),
        _ => None,
    }
}

fn spacing(value: &str) -> Option<Spacing> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("normal") {
        return Some(Spacing::Pixels(0.0));
    }
    let spacing = relative(value, Spacing::Ems, Spacing::Pixels).or_else(|| {
        let length = Length::parse(value).filter(|length| length.unit != Unit::Percent)?;
        Some(Spacing::Pixels(length.pixels(0.0)))
    })?;
    let finite = match spacing {
        Spacing::Pixels(amount) | Spacing::Ems(amount) => amount.is_finite(),
    };
    finite.then_some(spacing)
}

fn baseline_shift(value: &str) -> Option<BaselineShift> {
    let value = value.trim();
    let shift = match value.to_ascii_lowercase().as_str() {
        "baseline" => BaselineShift::Pixels(0.0),
        "sub" => BaselineShift::OfParentSize(SUBSCRIPT_SHIFT_EMS),
        "super" => BaselineShift::OfParentSize(SUPERSCRIPT_SHIFT_EMS),
        _ => relative(value, BaselineShift::Ems, BaselineShift::Pixels).or_else(|| {
            let length = Length::parse(value)?;
            Some(match length.unit {
                Unit::Percent => BaselineShift::OfParentSize(length.value / 100.0),
                _ => BaselineShift::Pixels(length.pixels(0.0)),
            })
        })?,
    };
    let finite = match shift {
        BaselineShift::Pixels(amount)
        | BaselineShift::Ems(amount)
        | BaselineShift::OfParentSize(amount) => amount.is_finite(),
    };
    finite.then_some(shift)
}

fn shorthand(value: &str) -> Option<Shorthand<'_>> {
    let value = value.trim();
    if SYSTEM_FONTS
        .iter()
        .any(|system| value.eq_ignore_ascii_case(system))
    {
        return None;
    }
    let mut weight = FontWeight::Absolute(NORMAL_WEIGHT);
    let mut slanted = false;
    let mut words = words(value).into_iter().peekable();
    let (size, mut after) = loop {
        let (at, word) = words.next()?;
        if word.parse::<f64>().is_ok() {
            weight = font_weight(word)?;
            continue;
        }
        let (size_text, line_height) = match word.split_once('/') {
            Some((size, line_height)) => (size, Some(line_height)),
            None => (word, None),
        };
        if let Some(size) = font_size(size_text) {
            let mut after = at + word.len();
            if line_height == Some("") {
                after = words.next().map_or(after, |(at, word)| at + word.len());
            } else if line_height.is_none()
                && let Some((at, next)) = words.peek().copied()
                && next.starts_with('/')
            {
                words.next();
                after = at + next.len();
                if next == "/" {
                    after = words.next().map_or(after, |(at, word)| at + word.len());
                }
            }
            break (size, after);
        }
        if let Some(chosen) = italic(word) {
            slanted = slanted || chosen;
        } else if let Some(chosen) = font_weight(word) {
            weight = chosen;
        } else if !SHORTHAND_KEYWORDS
            .iter()
            .any(|keyword| word.eq_ignore_ascii_case(keyword))
        {
            return None;
        }
    };
    after = after.min(value.len());
    let family = family(value.get(after..)?)?;
    Some(Shorthand {
        size,
        family,
        weight,
        italic: slanted,
    })
}

fn words(text: &str) -> Vec<(usize, &str)> {
    let mut words = Vec::new();
    let mut start = None;
    for (at, character) in text.char_indices().chain([(text.len(), ' ')]) {
        match (character.is_whitespace(), start) {
            (true, Some(from)) => {
                words.push((from, text.get(from..at).unwrap_or_default()));
                start = None;
            }
            (false, None) => start = Some(at),
            _ => {}
        }
    }
    words
}
