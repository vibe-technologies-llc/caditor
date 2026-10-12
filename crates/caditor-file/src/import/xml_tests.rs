use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use caditor_kernel::interruptible;

use crate::import::xml::{MAX_ATTRIBUTES, MAX_DEPTH, XmlError, parse};

const NO_LIMIT: usize = usize::MAX;

fn nested(levels: usize) -> String {
    format!("{}{}", "<g>".repeat(levels), "</g>".repeat(levels))
}

#[test]
fn a_hundred_thousand_distinct_namespaces_are_interned_without_scanning_them_all() {
    let siblings = 100_000;
    let mut text = String::from("<root>");
    for sibling in 0..siblings {
        text.push_str(&format!("<p:a xmlns:p=\"urn:namespace:{sibling}\"/>"));
    }
    text.push_str("</root>");
    let started = Instant::now();

    let tree = parse(&text, NO_LIMIT).unwrap();

    assert!(started.elapsed() < Duration::from_secs(10));
    let last = tree.nodes().last().unwrap();
    assert_eq!(last.namespace(), Some("urn:namespace:99999"));
    assert_eq!(tree.element_count(), siblings + 1);
}

#[test]
fn a_prefix_means_the_innermost_declaration_and_the_outer_one_again_after_it_closes() {
    let tree = parse(
        r#"<r xmlns:p="urn:outer"><x xmlns:p="urn:inner"><p:y/></x><p:z/><w xmlns:p=""><v/></w></r>"#,
        NO_LIMIT,
    )
    .unwrap();

    let named = |name: &str| tree.nodes().find(|node| node.name() == name).unwrap();

    assert_eq!(named("y").namespace(), Some("urn:inner"));
    assert_eq!(named("z").namespace(), Some("urn:outer"));
    assert_eq!(named("v").namespace(), None);
    assert_eq!(tree.damaged_at, None);
}

#[test]
fn a_prefix_is_undeclared_again_once_its_element_closes() {
    let tree = parse(r#"<r><x xmlns:p="urn:inner"/><p:y/></r>"#, NO_LIMIT).unwrap();

    assert_eq!(tree.damaged_at, Some(1));
    assert_eq!(tree.element_count(), 2);
}

#[test]
fn nesting_up_to_the_limit_is_read_and_one_level_more_is_refused() {
    assert!(parse(&nested(MAX_DEPTH), NO_LIMIT).is_ok());
    assert_eq!(
        parse(&nested(MAX_DEPTH + 1), NO_LIMIT).map(|_| ()),
        Err(XmlError::TooDeep)
    );
}

#[test]
fn attributes_up_to_the_limit_are_read_and_one_more_is_refused() {
    let tag = |attributes: usize| {
        let list: String = (0..attributes)
            .map(|attribute| format!(" a{attribute}=\"1\""))
            .collect();
        format!("<r{list}/>")
    };

    let at_the_limit = tag(MAX_ATTRIBUTES);
    let tree = parse(&at_the_limit, NO_LIMIT).unwrap();

    assert_eq!(tree.root().unwrap().attribute("a1023"), Some("1"));
    assert_eq!(
        parse(&tag(MAX_ATTRIBUTES + 1), NO_LIMIT).map(|_| ()),
        Err(XmlError::TooManyAttributes)
    );
}

#[test]
fn an_element_encloses_exactly_its_own_subtree() {
    let tree = parse("<r><a><b/><c><d/></c></a><e/></r>", NO_LIMIT).unwrap();
    let named = |name: &str| tree.nodes().find(|node| node.name() == name).unwrap();

    assert!(named("a").encloses(named("a")));
    assert!(named("a").encloses(named("d")));
    assert!(named("r").encloses(named("e")));
    assert!(!named("a").encloses(named("e")));
    assert!(!named("c").encloses(named("b")));
    assert!(!named("d").encloses(named("c")));
}

#[test]
fn elements_left_open_by_damage_enclose_everything_read_after_them() {
    let tree = parse("<r><a><b/><c>", NO_LIMIT).unwrap();
    let named = |name: &str| tree.nodes().find(|node| node.name() == name).unwrap();

    assert!(tree.damaged_at.is_some());
    assert!(named("r").encloses(named("c")));
    assert!(named("a").encloses(named("c")));
    assert!(!named("b").encloses(named("c")));
}

#[test]
fn a_cancelled_parse_stops_with_cancelled() {
    let text = format!("<root>{}</root>", "<a/>".repeat(10_000));

    let parsed = interruptible(Arc::new(|| true), || parse(&text, NO_LIMIT));

    assert_eq!(parsed.err(), Some(XmlError::Cancelled));
}
