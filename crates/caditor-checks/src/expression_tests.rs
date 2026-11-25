use std::{fs, path::Path};

use caditor_expression::Expression;

use crate::expression::{check_text, resolve};

fn seeds() -> Vec<String> {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fuzz/seeds/expression");
    let mut texts: Vec<String> = fs::read_dir(folder)
        .unwrap()
        .map(|entry| fs::read_to_string(entry.unwrap().path()).unwrap())
        .collect();

    texts.sort();
    texts
}

#[test]
fn every_expression_seed_holds_through_display_evaluation_and_scaling() {
    let texts = seeds();

    assert!(!texts.is_empty());

    for text in texts {
        check_text(&text);
    }
}

#[test]
fn every_typed_expression_seed_parses_with_the_parameters_the_target_knows() {
    for text in seeds().iter().filter(|text| !text.contains('$')) {
        assert!(Expression::parse(text, &resolve).is_ok(), "{text}");
    }
}
