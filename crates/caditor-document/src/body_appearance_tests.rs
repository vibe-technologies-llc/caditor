use caditor_expression::Expression;

use crate::{
    combine_tests::{Pair, pair},
    *,
};

const STEEL_BLUE: Rgb = Rgb::new(70, 130, 180);

fn steel(pair: &Pair, density: &str) -> BodyAppearance {
    BodyAppearance {
        colour: Some(STEEL_BLUE),
        material: Some("Steel".to_owned()),
        density: Some(pair.document.parse(density).unwrap()),
        name: None,
        opacity: None,
        faces: Vec::new(),
    }
}

fn set(pair: &Pair, appearance: BodyAppearance) -> Transaction {
    Transaction::single(
        "Appearance",
        Edit::SetBodyAppearance {
            id: pair.plate,
            appearance,
        },
    )
}

#[test]
fn hex_colours_read_back_as_written() {
    assert_eq!(STEEL_BLUE.hex(), "#4682b4");
    assert_eq!(Rgb::from_hex("#4682b4"), Some(STEEL_BLUE));
    assert_eq!(Rgb::from_hex(" 4682B4 "), Some(STEEL_BLUE));
    assert_eq!(Rgb::from_hex("#f80"), Some(Rgb::new(255, 136, 0)));

    assert_eq!(Rgb::from_hex("#4682b"), None);
    assert_eq!(Rgb::from_hex("#gg82b4"), None);
    assert_eq!(Rgb::from_hex("#+1+2+3"), None);
    assert_eq!(Rgb::from_hex("ÿÿÿ"), None);
}

#[test]
fn setting_an_appearance_is_undone_by_its_inverse() {
    let mut pair = pair();
    let before = pair.document.clone();
    let appearance = steel(&pair, "7.85");

    let inverse = pair.document.apply(set(&pair, appearance.clone())).unwrap();

    let plate = pair.document.feature(pair.plate).unwrap();
    assert_eq!(plate.appearance, appearance);
    assert!(!pair.document.same_content(&before));
    pair.document.apply(inverse).unwrap();
    assert!(pair.document.same_content(&before));
}

#[test]
fn a_material_name_is_trimmed_and_a_blank_one_dropped() {
    let mut pair = pair();

    let mut padded = steel(&pair, "7.85");
    padded.material = Some("  Brass ".to_owned());
    pair.document.apply(set(&pair, padded)).unwrap();
    let named = pair
        .document
        .feature(pair.plate)
        .unwrap()
        .appearance
        .clone();
    let mut blank = named.clone();
    blank.material = Some("   ".to_owned());
    pair.document.apply(set(&pair, blank)).unwrap();
    let unnamed = pair
        .document
        .feature(pair.plate)
        .unwrap()
        .appearance
        .clone();

    assert_eq!(named.material.as_deref(), Some("Brass"));
    assert_eq!(unnamed.material, None);
}

#[test]
fn an_overlong_material_name_is_refused() {
    let mut pair = pair();
    let mut appearance = steel(&pair, "7.85");
    appearance.material = Some("x".repeat(MAX_MATERIAL_NAME_CHARS + 1));

    let refused = pair.document.apply(set(&pair, appearance));

    assert_eq!(
        refused,
        Err(EditError::MaterialNameTooLong(MAX_MATERIAL_NAME_CHARS + 1))
    );
}

#[test]
fn only_a_feature_making_a_body_takes_an_appearance() {
    let mut pair = pair();
    let outline = pair
        .document
        .features()
        .find(|feature| feature.kind.sketch().is_some())
        .unwrap()
        .id();

    let refused = pair.document.apply(Transaction::single(
        "Appearance",
        Edit::SetBodyAppearance {
            id: outline,
            appearance: BodyAppearance::default(),
        },
    ));

    assert!(
        matches!(refused, Err(EditError::NotABody(_))),
        "{refused:?}"
    );
}

#[test]
fn a_density_parameter_counts_as_used_and_cannot_be_deleted() {
    let mut pair = pair();
    let mut transaction = pair.document.transaction("Add density");
    let density = transaction.add_parameter("rho", Expression::Number(2.7));
    pair.document.apply(transaction.finish()).unwrap();
    let appearance = steel(&pair, "rho");
    pair.document.apply(set(&pair, appearance)).unwrap();

    let users = pair.document.parameter_users(density);
    let removal = pair.document.apply(Transaction::single(
        "Delete",
        Edit::RemoveParameter { id: density },
    ));

    assert_eq!(users, vec!["Plate".to_owned()]);
    assert!(
        matches!(removal, Err(EditError::ParameterInUse { .. })),
        "{removal:?}"
    );
}

#[test]
fn an_appearance_naming_a_missing_parameter_is_refused() {
    let mut pair = pair();
    let mut other = pair.document.clone();
    let mut transaction = other.transaction("Add density");
    transaction.add_parameter("rho", Expression::Number(2.7));
    other.apply(transaction.finish()).unwrap();
    let appearance = BodyAppearance {
        density: Some(other.parse("rho").unwrap()),
        ..BodyAppearance::default()
    };

    let refused = pair.document.apply(set(&pair, appearance));

    assert_eq!(refused, Err(EditError::MissingParameter));
}

#[test]
fn deleting_and_restoring_a_body_keeps_its_appearance() {
    let mut pair = pair();
    let appearance = steel(&pair, "7.85");
    pair.document.apply(set(&pair, appearance.clone())).unwrap();
    let deletion = pair.document.deletion(&[pair.plate], "Delete");

    let inverse = pair.document.apply(deletion).unwrap();
    pair.document.apply(inverse).unwrap();

    assert_eq!(
        pair.document.feature(pair.plate).unwrap().appearance,
        appearance
    );
}

#[test]
fn mass_is_the_volume_times_the_density_in_grams() {
    let pair = pair();
    let values = ParameterValues::evaluate(&pair.document);
    let appearance = steel(&pair, "7.85");
    let cubic_centimetre = 1000.0;

    let mass = appearance.mass_grams(cubic_centimetre, &values);

    assert!(
        matches!(mass, Some(Ok(grams)) if (grams - 7.85).abs() < 1e-12),
        "{mass:?}"
    );
    assert_eq!(BodyAppearance::default().mass_grams(1.0, &values), None);
}

#[test]
fn a_density_that_is_not_a_plain_positive_number_says_why() {
    let pair = pair();
    let values = ParameterValues::evaluate(&pair.document);
    let density = |text: &str| steel(&pair, text).density_value(&values).unwrap();

    assert_eq!(density("0"), Err(DensityError::NotAboveZero(0.0)));
    assert_eq!(density("-1"), Err(DensityError::NotAboveZero(-1.0)));
    assert_eq!(density("500"), Err(DensityError::TooHigh(500.0)));
    assert!(matches!(density("5 mm"), Err(DensityError::Evaluation(_))));
    assert_eq!(density("2 * 1.35"), Ok(2.7));
}

#[test]
fn a_density_follows_its_parameter() {
    let mut pair = pair();
    let mut transaction = pair.document.transaction("Add density");
    transaction.add_parameter("rho", Expression::Number(2.7));
    pair.document.apply(transaction.finish()).unwrap();
    let appearance = steel(&pair, "rho * 2");
    let values = ParameterValues::evaluate(&pair.document);

    let density = appearance.density_value(&values);

    assert_eq!(density, Some(Ok(5.4)));
}

#[test]
fn a_body_may_be_see_through_down_to_a_tenth_and_full_opacity_is_stored_as_none() {
    let mut pair = pair();
    let faded = |opacity| BodyAppearance {
        opacity: Some(opacity),
        ..BodyAppearance::default()
    };

    pair.document.apply(set(&pair, faded(40))).unwrap();
    let kept = pair
        .document
        .feature(pair.plate)
        .unwrap()
        .appearance
        .opacity;
    pair.document.apply(set(&pair, faded(100))).unwrap();
    let solid = pair
        .document
        .feature(pair.plate)
        .unwrap()
        .appearance
        .opacity;
    let refused = pair.document.apply(set(&pair, faded(5)));

    assert_eq!(kept, Some(40));
    assert_eq!(solid, None);
    assert_eq!(refused, Err(EditError::OpacityTooLow(5)));
}

#[test]
fn an_opacity_snaps_to_the_nearest_step_and_solid_is_the_step_above_the_first() {
    let snapped: Vec<Option<u8>> = [100, 88, 87, 80, 62, 60, 40, 37, 12, 0]
        .into_iter()
        .map(nearest_opacity_step)
        .collect();

    assert_eq!(
        snapped,
        [
            None,
            None,
            Some(75),
            Some(75),
            Some(50),
            Some(50),
            Some(50),
            Some(25),
            Some(25),
            Some(25)
        ]
    );
}

#[test]
fn a_face_colour_follows_the_face_into_every_fragment_a_later_cut_leaves() {
    use caditor_geometry::Vector3;
    use caditor_kernel::{FaceReference, Surface};

    use crate::combine_tests::{evaluate, rectangle};

    let mut pair = pair();
    let mut engine = Recompute::default();
    let before = evaluate(&pair.document, &mut engine);
    let solid = before.body(pair.plate).unwrap();
    let top = solid
        .faces()
        .find(|(_, face)| match face.surface() {
            Surface::Plane(plane) => {
                (plane.frame().normal() * face.sense().sign()).distance(Vector3::Z) < 1e-9
            }
            _ => false,
        })
        .map(|(id, _)| id)
        .unwrap();
    let red = Rgb::new(200, 64, 52);
    let appearance = BodyAppearance {
        faces: vec![FaceColour {
            face: FaceReference::capture(solid, top).unwrap(),
            colour: red,
        }],
        ..BodyAppearance::default()
    };
    pair.document.apply(set(&pair, appearance)).unwrap();
    let mut transaction = pair.document.transaction("Slot");
    let slot = transaction.add_feature(
        "Slot sketch",
        FeatureKind::from(rectangle((8.0, -1.0), (12.0, 11.0))),
    );
    transaction.add_feature(
        "Slot",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch: slot,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(Expression::parse_stored("10 mm").unwrap(), false),
            operation: BodyOperation::Remove(pair.plate),
            start: None,
            other_bodies: Vec::new(),
        })),
    );
    pair.document.apply(transaction.finish()).unwrap();

    let after = evaluate(&pair.document, &mut engine);
    let coloured = pair
        .document
        .feature(pair.plate)
        .unwrap()
        .appearance
        .face_colours(after.body(pair.plate).unwrap());

    assert_eq!(after.failed_count(), 0);
    assert_eq!(coloured.len(), 2);
    assert!(coloured.values().all(|colour| *colour == red));
}
