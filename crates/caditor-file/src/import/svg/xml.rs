use std::{borrow::Cow, collections::BTreeMap};

pub(super) const XML_NAMESPACE: &str = "http://www.w3.org/XML/1998/namespace";
const MAX_ENTITY_DEPTH: usize = 8;
const MAX_ENTITY_WORK: usize = 4 << 20;
const TEXT_KEPT_IN: &str = "style";
const CONTENT_KEPT_IN: [&str; 4] = ["text", "tspan", "textPath", "a"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum XmlError {
    TooManyElements,
    Damaged { line: usize },
}

#[derive(Debug)]
pub(super) struct Tree<'t> {
    elements: Vec<Element<'t>>,
    namespaces: Vec<Cow<'t, str>>,
    pub damaged_at: Option<usize>,
}

#[derive(Debug)]
struct Element<'t> {
    namespace: Option<usize>,
    name: &'t str,
    attributes: Vec<Attribute<'t>>,
    parent: Option<usize>,
    children: Vec<usize>,
    text: Cow<'t, str>,
    pieces: Vec<Piece<'t>>,
}

#[derive(Debug)]
struct Piece<'t> {
    before_child: usize,
    text: Cow<'t, str>,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum Content<'a, 't> {
    Text(&'a str),
    Element(Node<'a, 't>),
}

#[derive(Debug)]
struct Attribute<'t> {
    namespace: Option<usize>,
    name: &'t str,
    value: Cow<'t, str>,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Node<'a, 't> {
    tree: &'a Tree<'t>,
    index: usize,
}

impl<'a, 't> Node<'a, 't> {
    fn element(self) -> Option<&'a Element<'t>> {
        self.tree.elements.get(self.index)
    }

    pub fn id(self) -> usize {
        self.index
    }

    pub fn name(self) -> &'t str {
        self.element().map_or("", |element| element.name)
    }

    pub fn namespace(self) -> Option<&'a str> {
        self.tree.namespace(self.element()?.namespace)
    }

    pub fn attribute(self, name: &str) -> Option<&'a str> {
        self.element()?
            .attributes
            .iter()
            .find(|attribute| attribute.namespace.is_none() && attribute.name == name)
            .map(|attribute| attribute.value.as_ref())
    }

    pub fn attribute_in(self, namespace: &str, name: &str) -> Option<&'a str> {
        self.element()?
            .attributes
            .iter()
            .find(|attribute| {
                attribute.name == name
                    && self.tree.namespace(attribute.namespace) == Some(namespace)
            })
            .map(|attribute| attribute.value.as_ref())
    }

    pub fn children(self) -> impl Iterator<Item = Node<'a, 't>> + Clone {
        let tree = self.tree;
        self.element()
            .map(|element| element.children.as_slice())
            .unwrap_or_default()
            .iter()
            .map(move |index| Node {
                tree,
                index: *index,
            })
    }

    pub fn text(self) -> &'a str {
        self.element().map_or("", |element| element.text.as_ref())
    }

    pub fn content(self) -> Vec<Content<'a, 't>> {
        let Some(element) = self.element() else {
            return Vec::new();
        };
        let tree = self.tree;
        let mut pieces = element.pieces.iter().peekable();
        let mut content = Vec::with_capacity(element.children.len() + element.pieces.len());
        for (position, index) in element.children.iter().enumerate() {
            while let Some(piece) = pieces.next_if(|piece| piece.before_child <= position) {
                content.push(Content::Text(piece.text.as_ref()));
            }
            content.push(Content::Element(Node {
                tree,
                index: *index,
            }));
        }
        content.extend(pieces.map(|piece| Content::Text(piece.text.as_ref())));
        content
    }

    pub fn ancestors(self) -> impl Iterator<Item = Node<'a, 't>> {
        let tree = self.tree;
        std::iter::successors(Some(self), move |node| {
            node.element()?.parent.map(|index| Node { tree, index })
        })
    }
}

impl<'t> Tree<'t> {
    pub fn root(&self) -> Option<Node<'_, 't>> {
        (!self.elements.is_empty()).then_some(Node {
            tree: self,
            index: 0,
        })
    }

    pub fn nodes(&self) -> impl Iterator<Item = Node<'_, 't>> {
        (0..self.elements.len()).map(|index| Node { tree: self, index })
    }

    fn namespace(&self, index: Option<usize>) -> Option<&str> {
        self.namespaces.get(index?).map(AsRef::as_ref)
    }
}

pub(super) fn parse(text: &str, max_elements: usize) -> Result<Tree<'_>, XmlError> {
    let mut parser = Parser {
        text,
        at: 0,
        entities: BTreeMap::new(),
        entity_work: 0,
        scopes: Vec::new(),
        open: Vec::new(),
        tree: Tree {
            elements: Vec::new(),
            namespaces: Vec::new(),
            damaged_at: None,
        },
        max_elements,
    };
    let ended = parser.run()?;
    let damage = match ended {
        Ended::Whole => None,
        Ended::DamagedAt(at) => Some(at),
    };
    let damaged_at = damage.map(|at| line_of(text, at));
    if parser.tree.elements.is_empty() {
        return match damaged_at {
            Some(line) => Err(XmlError::Damaged { line }),
            None => Ok(parser.tree),
        };
    }
    parser.tree.damaged_at = damaged_at;
    Ok(parser.tree)
}

fn line_of(text: &str, at: usize) -> usize {
    text.as_bytes()
        .get(..at)
        .unwrap_or(text.as_bytes())
        .iter()
        .filter(|byte| **byte == b'\n')
        .count()
        + 1
}

enum Ended {
    Whole,
    DamagedAt(usize),
}

struct Scope<'t> {
    element: usize,
    prefixes: Vec<(&'t str, usize)>,
}

struct Parser<'t> {
    text: &'t str,
    at: usize,
    entities: BTreeMap<&'t str, &'t str>,
    entity_work: usize,
    scopes: Vec<Scope<'t>>,
    open: Vec<(usize, &'t str)>,
    tree: Tree<'t>,
    max_elements: usize,
}

impl<'t> Parser<'t> {
    fn rest(&self) -> &'t str {
        self.text.get(self.at..).unwrap_or_default()
    }

    fn run(&mut self) -> Result<Ended, XmlError> {
        loop {
            let rest = self.rest();
            if rest.is_empty() {
                return Ok(if self.open.is_empty() && !self.tree.elements.is_empty() {
                    Ended::Whole
                } else {
                    Ended::DamagedAt(self.at)
                });
            }
            let start = self.at;
            let step = if rest.starts_with('<') {
                self.markup(rest)?
            } else {
                let length = rest.find('<').unwrap_or(rest.len());
                self.keep_text(rest.get(..length).unwrap_or_default(), true);
                Some(Step::Skip(length))
            };
            match step {
                Some(Step::Skip(length)) => self.at += length,
                Some(Step::Closed(length)) => {
                    self.at += length;
                    if self.open.is_empty() {
                        return Ok(Ended::Whole);
                    }
                }
                None => return Ok(Ended::DamagedAt(start)),
            }
        }
    }

    fn markup(&mut self, rest: &'t str) -> Result<Option<Step>, XmlError> {
        let through = |marker: &str, end: &str| {
            rest.get(marker.len()..)
                .and_then(|body| body.find(end))
                .map(|found| Step::Skip(marker.len() + found + end.len()))
        };
        if rest.starts_with("<!--") {
            return Ok(through("<!--", "-->"));
        }
        if let Some(body) = rest.strip_prefix("<![CDATA[") {
            if let Some(end) = body.find("]]>") {
                self.keep_text(body.get(..end).unwrap_or_default(), false);
            }
            return Ok(through("<![CDATA[", "]]>"));
        }
        if rest.starts_with("<?") {
            return Ok(through("<?", "?>"));
        }
        if rest.starts_with("<!DOCTYPE") {
            return Ok(self.doctype(rest).map(Step::Skip));
        }
        if rest.starts_with("<!") {
            return Ok(None);
        }
        if let Some(body) = rest.strip_prefix("</") {
            return Ok(self.end_tag(body));
        }
        self.start_tag(rest)
    }

    fn end_tag(&mut self, body: &'t str) -> Option<Step> {
        let close = body.find('>')?;
        let name = body.get(..close)?.trim_end();
        let (_, open) = self.open.last()?;
        if *open != name {
            return None;
        }
        let (element, _) = self.open.pop()?;
        if self
            .scopes
            .last()
            .is_some_and(|scope| scope.element == element)
        {
            self.scopes.pop();
        }
        Some(Step::Closed(close + 3))
    }

    fn start_tag(&mut self, rest: &'t str) -> Result<Option<Step>, XmlError> {
        let Some(tag) = Tag::read(rest) else {
            return Ok(None);
        };
        if self.open.is_empty() && !self.tree.elements.is_empty() {
            return Ok(None);
        }
        if self.tree.elements.len() >= self.max_elements {
            return Err(XmlError::TooManyElements);
        }
        let index = self.tree.elements.len();
        let mut prefixes = Vec::new();
        let mut plain = Vec::new();
        for (name, raw) in tag.attributes {
            let Some(value) = self.decoded(raw) else {
                return Ok(None);
            };
            if name == "xmlns" {
                prefixes.push(("", self.intern(value)));
            } else if let Some(prefix) = name.strip_prefix("xmlns:") {
                prefixes.push((prefix, self.intern(value)));
            } else {
                plain.push((name, value));
            }
        }
        if !prefixes.is_empty() {
            self.scopes.push(Scope {
                element: index,
                prefixes,
            });
        }
        let Some((namespace, name)) = self.resolved(tag.name, true) else {
            return Ok(None);
        };
        let mut attributes = Vec::with_capacity(plain.len());
        for (qualified, value) in plain {
            let Some((namespace, name)) = self.resolved(qualified, false) else {
                return Ok(None);
            };
            attributes.push(Attribute {
                namespace,
                name,
                value,
            });
        }
        let parent = self.open.last().map(|(parent, _)| *parent);
        if let Some(parent) = parent.and_then(|parent| self.tree.elements.get_mut(parent)) {
            parent.children.push(index);
        }
        self.tree.elements.push(Element {
            namespace,
            name,
            attributes,
            parent,
            children: Vec::new(),
            text: Cow::Borrowed(""),
            pieces: Vec::new(),
        });
        if tag.empty {
            if self
                .scopes
                .last()
                .is_some_and(|scope| scope.element == index)
            {
                self.scopes.pop();
            }
            if self.open.is_empty() {
                return Ok(Some(Step::Closed(tag.length)));
            }
        } else {
            self.open.push((index, tag.name));
        }
        Ok(Some(Step::Skip(tag.length)))
    }

    fn keep_text(&mut self, raw: &'t str, escaped: bool) {
        let Some((element, name)) = self.open.last().copied() else {
            return;
        };
        let local = name.rsplit(':').next().unwrap_or(name);
        let is_style = local == TEXT_KEPT_IN;
        if !is_style && !CONTENT_KEPT_IN.contains(&local) {
            return;
        }
        let text = if escaped {
            self.decoded(raw).unwrap_or(Cow::Borrowed(raw))
        } else {
            Cow::Borrowed(raw)
        };
        let Some(element) = self.tree.elements.get_mut(element) else {
            return;
        };
        if is_style {
            if element.text.is_empty() {
                element.text = text;
            } else {
                element.text.to_mut().push_str(&text);
            }
            return;
        }
        let before_child = element.children.len();
        match element.pieces.last_mut() {
            Some(last) if last.before_child == before_child => last.text.to_mut().push_str(&text),
            _ => element.pieces.push(Piece { before_child, text }),
        }
    }

    fn intern(&mut self, value: Cow<'t, str>) -> usize {
        match self
            .tree
            .namespaces
            .iter()
            .position(|known| *known == value)
        {
            Some(index) => index,
            None => {
                self.tree.namespaces.push(value);
                self.tree.namespaces.len() - 1
            }
        }
    }

    fn resolved(&mut self, qualified: &'t str, element: bool) -> Option<(Option<usize>, &'t str)> {
        let (prefix, name) = match qualified.split_once(':') {
            Some((prefix, name)) => (prefix, name),
            None if element => ("", qualified),
            None => return Some((None, qualified)),
        };
        if prefix == "xml" {
            return Some((Some(self.intern(Cow::Borrowed(XML_NAMESPACE))), name));
        }
        let bound = self
            .scopes
            .iter()
            .rev()
            .flat_map(|scope| scope.prefixes.iter().rev())
            .find(|(known, _)| *known == prefix)
            .map(|(_, uri)| *uri);
        let declared = bound.filter(|uri| {
            self.tree
                .namespace(Some(*uri))
                .is_some_and(|uri| !uri.is_empty())
        });
        match (declared, prefix.is_empty()) {
            (Some(uri), _) => Some((Some(uri), name)),
            (None, true) => Some((None, name)),
            (None, false) => None,
        }
    }

    fn decoded(&mut self, raw: &'t str) -> Option<Cow<'t, str>> {
        if raw.contains('<') {
            return None;
        }
        if !raw.contains(['&', '\t', '\n', '\r']) {
            return Some(Cow::Borrowed(raw));
        }
        let mut value = String::with_capacity(raw.len());
        expand(raw, &self.entities, &mut self.entity_work, 0, &mut value)?;
        Some(Cow::Owned(value))
    }

    fn doctype(&mut self, rest: &'t str) -> Option<usize> {
        let mut at = "<!DOCTYPE".len();
        loop {
            let byte = *rest.as_bytes().get(at)?;
            match byte {
                b'>' => return Some(at + 1),
                b'[' => break,
                b'"' | b'\'' => at += quoted_length(rest.get(at..)?)?,
                _ => at += 1,
            }
        }
        at += 1;
        loop {
            let subset = rest.get(at..)?;
            let trimmed = subset.trim_start();
            at += subset.len() - trimmed.len();
            if let Some(after) = trimmed.strip_prefix(']') {
                let closing = after.find('>')?;
                return Some(at + 1 + closing + 1);
            }
            if trimmed.starts_with("<!--") {
                at += trimmed.find("-->")? + 3;
            } else if trimmed.starts_with("<?") {
                at += trimmed.find("?>")? + 2;
            } else if let Some(declaration) = trimmed.strip_prefix("<!ENTITY") {
                self.entity(declaration);
                at += declaration_length(trimmed)?;
            } else if trimmed.starts_with("<!") {
                at += declaration_length(trimmed)?;
            } else if trimmed.starts_with('%') {
                at += trimmed.find(';')? + 1;
            } else {
                return None;
            }
        }
    }

    fn entity(&mut self, declaration: &'t str) {
        let declaration = declaration.trim_start();
        if declaration.starts_with('%') {
            return;
        }
        let name_end = declaration
            .find(|character: char| character.is_whitespace())
            .unwrap_or(declaration.len());
        let (Some(name), Some(after)) = (declaration.get(..name_end), declaration.get(name_end..))
        else {
            return;
        };
        let after = after.trim_start();
        let Some(quote) = after
            .chars()
            .next()
            .filter(|quote| *quote == '"' || *quote == '\'')
        else {
            return;
        };
        let Some(value) = after
            .get(1..)
            .and_then(|body| body.find(quote).and_then(|end| body.get(..end)))
        else {
            return;
        };
        self.entities.entry(name).or_insert(value);
    }
}

enum Step {
    Skip(usize),
    Closed(usize),
}

struct Tag<'t> {
    name: &'t str,
    attributes: Vec<(&'t str, &'t str)>,
    empty: bool,
    length: usize,
}

impl<'t> Tag<'t> {
    fn read(rest: &'t str) -> Option<Self> {
        let mut at = 1;
        let name = name_at(rest, at)?;
        at += name.len();
        let mut attributes = Vec::new();
        loop {
            let after = rest.get(at..)?;
            let trimmed = after.trim_start();
            let spaced = trimmed.len() < after.len();
            at += after.len() - trimmed.len();
            if trimmed.starts_with("/>") {
                return Some(Self {
                    name,
                    attributes,
                    empty: true,
                    length: at + 2,
                });
            }
            if trimmed.starts_with('>') {
                return Some(Self {
                    name,
                    attributes,
                    empty: false,
                    length: at + 1,
                });
            }
            if !spaced {
                return None;
            }
            let attribute = name_at(rest, at)?;
            at += attribute.len();
            let after = rest.get(at..)?;
            let trimmed = after.trim_start().strip_prefix('=')?.trim_start();
            at += after.len() - trimmed.len();
            let length = quoted_length(trimmed)?;
            attributes.push((attribute, trimmed.get(1..length - 1)?));
            at += length;
        }
    }
}

fn name_at(text: &str, at: usize) -> Option<&str> {
    let rest = text.get(at..)?;
    let end = rest
        .find(|character: char| {
            character.is_whitespace() || matches!(character, '/' | '>' | '=' | '<' | '"' | '\'')
        })
        .unwrap_or(rest.len());
    rest.get(..end).filter(|name| !name.is_empty())
}

fn quoted_length(text: &str) -> Option<usize> {
    let quote = text
        .chars()
        .next()
        .filter(|quote| *quote == '"' || *quote == '\'')?;
    Some(text.get(1..)?.find(quote)? + 2)
}

fn declaration_length(text: &str) -> Option<usize> {
    let mut at = 0;
    loop {
        let byte = *text.as_bytes().get(at)?;
        match byte {
            b'>' => return Some(at + 1),
            b'"' | b'\'' => at += quoted_length(text.get(at..)?)?,
            _ => at += 1,
        }
    }
}

fn expand(
    raw: &str,
    entities: &BTreeMap<&str, &str>,
    work: &mut usize,
    depth: usize,
    value: &mut String,
) -> Option<()> {
    let mut rest = raw;
    while let Some(at) = rest.find(['&', '\t', '\n', '\r']) {
        value.push_str(rest.get(..at)?);
        let marker = rest.get(at..)?;
        if !marker.starts_with('&') {
            value.push(' ');
            rest = marker.get(1..)?;
            continue;
        }
        let end = marker.find(';')?;
        let name = marker.get(1..end)?;
        rest = marker.get(end + 1..)?;
        *work = work.saturating_add(name.len() + 1);
        match name {
            "lt" => value.push('<'),
            "gt" => value.push('>'),
            "amp" => value.push('&'),
            "quot" => value.push('"'),
            "apos" => value.push('\''),
            _ => {
                if let Some(code) = name.strip_prefix('#') {
                    let number = match code.strip_prefix('x') {
                        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                        None => code.parse::<u32>().ok()?,
                    };
                    value.push(char::from_u32(number)?);
                } else {
                    let replacement = entities.get(name)?;
                    if depth >= MAX_ENTITY_DEPTH || replacement.contains('<') {
                        return None;
                    }
                    *work = work.saturating_add(replacement.len());
                    if *work > MAX_ENTITY_WORK {
                        return None;
                    }
                    expand(replacement, entities, work, depth + 1, value)?;
                }
            }
        }
        if *work > MAX_ENTITY_WORK {
            return None;
        }
    }
    value.push_str(rest);
    Some(())
}
