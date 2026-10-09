use std::cell::Cell;

use crate::{import::svg::xml::Node, untrusted::UntrustedMap};

pub(super) const MAX_STYLE_RULES: usize = 10_000;
const MAX_MATCHING_WORK: usize = 50_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Precedence {
    important: bool,
    inline: bool,
    specificity: (usize, usize, usize),
    order: usize,
    position: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Declaration<'a> {
    pub name: &'a str,
    pub value: &'a str,
    pub important: bool,
}

pub(super) fn inline_declarations(style: &str) -> Vec<(Precedence, Declaration<'_>)> {
    declarations(style)
        .into_iter()
        .enumerate()
        .map(|(position, declaration)| {
            let precedence = Precedence {
                important: declaration.important,
                inline: true,
                specificity: (0, 0, 0),
                order: 0,
                position,
            };
            (precedence, declaration)
        })
        .collect()
}

fn declarations(block: &str) -> Vec<Declaration<'_>> {
    split_outside_brackets(block, ';')
        .filter_map(|declaration| declaration.split_once(':'))
        .filter_map(|(name, value)| {
            let name = name.trim();
            let value = value.trim();
            let (value, important) = match without_important(value) {
                Some(value) => (value, true),
                None => (value, false),
            };
            (!name.is_empty()).then_some(Declaration {
                name,
                value,
                important,
            })
        })
        .collect()
}

fn without_important(value: &str) -> Option<&str> {
    let length = value.len().checked_sub("important".len())?;
    let (rest, word) = (value.get(..length)?, value.get(length..)?);
    if !word.eq_ignore_ascii_case("important") {
        return None;
    }
    Some(rest.trim_end().strip_suffix('!')?.trim_end())
}

fn split_outside_brackets(text: &str, separator: char) -> impl Iterator<Item = &str> {
    let mut depth = 0_usize;
    let mut quote = None;
    text.split(move |character: char| {
        match (quote, character) {
            (Some(open), _) if character == open => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(character),
            (None, '(' | '[') => depth += 1,
            (None, ')' | ']') => depth = depth.saturating_sub(1),
            _ => {}
        }
        quote.is_none() && depth == 0 && character == separator
    })
}

#[derive(Debug, Default)]
struct Compound<'a> {
    element: Option<&'a str>,
    id: Option<&'a str>,
    classes: Vec<&'a str>,
}

impl<'a> Compound<'a> {
    fn parse(text: &'a str) -> Option<Self> {
        let mut compound = Self::default();
        let mut rest = text;
        if let Some(after) = rest.strip_prefix('*') {
            rest = after;
        } else if let Some((name, after)) = identifier(rest) {
            compound.element = Some(name);
            rest = after;
        }
        while !rest.is_empty() {
            if let Some((class, after)) = rest.strip_prefix('.').and_then(identifier) {
                compound.classes.push(class);
                rest = after;
            } else if let Some((id, after)) = rest.strip_prefix('#').and_then(identifier)
                && compound.id.is_none()
            {
                compound.id = Some(id);
                rest = after;
            } else {
                return None;
            }
        }
        (!text.is_empty()).then_some(compound)
    }

    fn specificity(&self) -> (usize, usize, usize) {
        (
            usize::from(self.id.is_some()),
            self.classes.len(),
            usize::from(self.element.is_some()),
        )
    }

    fn matches(&self, node: Node<'_, '_>) -> bool {
        self.element.is_none_or(|name| node.name() == name)
            && self
                .id
                .is_none_or(|id| node.attribute("id").map(str::trim) == Some(id))
            && self
                .classes
                .iter()
                .all(|class| classes(node).any(|own| own == *class))
    }
}

fn identifier(text: &str) -> Option<(&str, &str)> {
    let end = text
        .char_indices()
        .find(|(_, character)| {
            !(character.is_ascii_alphanumeric()
                || matches!(character, '-' | '_')
                || !character.is_ascii())
        })
        .map_or(text.len(), |(at, _)| at);
    let name = text.get(..end).filter(|name| !name.is_empty())?;
    Some((name, text.get(end..)?))
}

fn classes<'a>(node: Node<'a, '_>) -> impl Iterator<Item = &'a str> {
    node.attribute("class")
        .unwrap_or_default()
        .split_ascii_whitespace()
}

#[derive(Debug)]
struct Rule<'a> {
    selector: Compound<'a>,
    block: usize,
}

#[derive(Debug, Default)]
pub(super) struct StyleSheet<'a> {
    rules: Vec<Rule<'a>>,
    blocks: Vec<Vec<Declaration<'a>>>,
    by_id: UntrustedMap<&'a str, Vec<usize>>,
    by_class: UntrustedMap<&'a str, Vec<usize>>,
    by_element: UntrustedMap<&'a str, Vec<usize>>,
    universal: Vec<usize>,
    work: Cell<usize>,
    pub unread_selectors: usize,
    pub beyond_the_limit: usize,
}

impl<'a> StyleSheet<'a> {
    pub fn parse(text: &'a str) -> Self {
        let mut sheet = Self::default();
        let mut rest = text;
        loop {
            rest = rest.trim_start();
            if rest.is_empty() {
                return sheet;
            }
            if let Some(after) = ["<!--", "-->"]
                .into_iter()
                .find_map(|marker| rest.strip_prefix(marker))
            {
                rest = after;
                continue;
            }
            let Some(open) = rest.find(['{', ';']) else {
                return sheet;
            };
            let at_rule = rest.starts_with('@');
            if rest.as_bytes().get(open) == Some(&b';') {
                if !at_rule {
                    sheet.unread_selectors += 1;
                }
                rest = rest.get(open + 1..).unwrap_or_default();
                continue;
            }
            let close = block_end(rest, open);
            if !at_rule {
                let prelude = rest.get(..open).unwrap_or_default();
                let block = rest
                    .get(open + 1..close.unwrap_or(rest.len()))
                    .unwrap_or_default();
                sheet.rule(prelude, block);
            }
            rest = close
                .and_then(|close| rest.get(close + 1..))
                .unwrap_or_default();
        }
    }

    fn rule(&mut self, prelude: &'a str, block: &'a str) {
        let index = self.blocks.len();
        let mut used = false;
        for selector in split_outside_brackets(prelude, ',') {
            let Some(selector) = Compound::parse(selector.trim()) else {
                self.unread_selectors += 1;
                continue;
            };
            if self.rules.len() >= MAX_STYLE_RULES {
                self.beyond_the_limit += 1;
                continue;
            }
            let order = self.rules.len();
            let bucket = if let Some(id) = selector.id {
                self.by_id.entry(id).or_default()
            } else if let Some(class) = selector.classes.first() {
                self.by_class.entry(*class).or_default()
            } else if let Some(element) = selector.element {
                self.by_element.entry(element).or_default()
            } else {
                &mut self.universal
            };
            bucket.push(order);
            self.rules.push(Rule {
                selector,
                block: index,
            });
            used = true;
        }
        if used {
            self.blocks.push(declarations(block));
        }
    }

    pub fn exhausted(&self) -> bool {
        self.work.get() > MAX_MATCHING_WORK
    }

    pub fn declarations(&self, node: Node<'_, '_>) -> Vec<(Precedence, Declaration<'a>)> {
        if self.rules.is_empty() || self.exhausted() {
            return Vec::new();
        }
        let id = node.attribute("id").map(str::trim);
        let mut candidates: Vec<usize> = self.universal.clone();
        let mut add = |bucket: Option<&Vec<usize>>| {
            candidates.extend(bucket.into_iter().flatten());
        };
        add(id.and_then(|id| self.by_id.get(id)));
        add(self.by_element.get(node.name()));
        for class in classes(node) {
            add(self.by_class.get(class));
        }
        candidates.sort_unstable();
        candidates.dedup();
        self.work
            .set(self.work.get().saturating_add(candidates.len()));
        if self.exhausted() {
            return Vec::new();
        }
        candidates
            .into_iter()
            .filter_map(|order| Some((order, self.rules.get(order)?)))
            .filter(|(_, rule)| rule.selector.matches(node))
            .flat_map(|(order, rule)| {
                let specificity = rule.selector.specificity();
                self.blocks
                    .get(rule.block)
                    .into_iter()
                    .flatten()
                    .enumerate()
                    .map(move |(position, declaration)| {
                        let precedence = Precedence {
                            important: declaration.important,
                            inline: false,
                            specificity,
                            order,
                            position,
                        };
                        (precedence, *declaration)
                    })
            })
            .collect()
    }
}

fn block_end(text: &str, open: usize) -> Option<usize> {
    let mut depth = 0_usize;
    let mut quote = None;
    for (at, character) in text.char_indices().skip_while(|(at, _)| *at < open) {
        match (quote, character) {
            (Some(open), _) if character == open => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(character),
            (None, '{') => depth += 1,
            (None, '}') => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(at);
                }
            }
            _ => {}
        }
    }
    None
}

pub(super) fn without_comments(text: &str, sheet: &mut String) {
    let mut rest = text;
    while let Some(start) = rest.find("/*") {
        sheet.push_str(rest.get(..start).unwrap_or_default());
        sheet.push(' ');
        rest = match rest
            .get(start + 2..)
            .and_then(|body| body.find("*/").and_then(|end| body.get(end + 2..)))
        {
            Some(after) => after,
            None => return,
        };
    }
    sheet.push_str(rest);
    sheet.push('\n');
}
