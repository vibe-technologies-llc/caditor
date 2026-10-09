use caditor_expression::{Expression, ParameterId, Quantity};
use caditor_geometry::{Plane, Point2, Point3};
use caditor_kernel::{EdgeReference, Solid};
use caditor_sketch::Sketch;

use crate::*;

fn rectangle(min: (f64, f64), max: (f64, f64)) -> Sketch {
    let mut sketch = Sketch::new(Plane::XY);
    let corners = [
        Point2::new(min.0, min.1),
        Point2::new(max.0, min.1),
        Point2::new(max.0, max.1),
        Point2::new(min.0, max.1),
    ];
    for index in 0..4 {
        sketch.add_line(corners[index], corners[(index + 1) % 4]);
    }
    sketch
}

fn extrude(sketch: FeatureId, height: Expression) -> FeatureKind {
    FeatureKind::Solid(SolidFeature::Extrude(Extrude {
        sketch,
        regions: RegionChoice::All,
        extent: ExtrudeExtent::one_side(height, false),
        operation: BodyOperation::NewBody,
        start: None,
        other_bodies: Vec::new(),
        taper: None,
        wall: None,
    }))
}

struct Built {
    document: Document,
    height: ParameterId,
    outline: FeatureId,
    base: FeatureId,
}

fn built() -> Built {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let height = transaction.add_parameter("height", transaction.parse("4 mm").unwrap());
    let outline = transaction.add_feature(
        "Outline",
        FeatureKind::from(rectangle((0.0, 0.0), (10.0, 8.0))),
    );
    let base = transaction.add_feature("Base", extrude(outline, Expression::Parameter(height)));
    document.apply(transaction.finish()).unwrap();
    Built {
        document,
        height,
        outline,
        base,
    }
}

fn copied(document: &Document, ids: &[FeatureId]) -> Vec<Feature> {
    ids.iter()
        .map(|id| document.feature(*id).unwrap().clone())
        .collect()
}

fn carried(document: &Document, features: &[Feature]) -> CarriedParameters {
    let values = ParameterValues::evaluate(document);
    CarriedParameters::of(
        document,
        &values,
        features.iter().flat_map(feature_parameters),
    )
}

#[test]
fn pasted_features_get_fresh_ids_and_follow_each_other_not_the_originals() {
    let Built {
        mut document,
        height,
        outline,
        base,
    } = built();
    let features = copied(&document, &[outline, base]);
    let parameters = carried(&document, &features);

    let paste = document
        .paste_features(&features, &parameters, PasteOrigin::ThisDocument)
        .unwrap();
    document.apply(paste.transaction.clone()).unwrap();

    let [sketch, solid] = paste.pasted[..] else {
        panic!("two features should be pasted: {:?}", paste.pasted);
    };
    assert!(paste.left_out.is_empty());
    assert_eq!(paste.inlined, 0);
    assert!(![outline, base].contains(&sketch) && ![outline, base].contains(&solid));
    assert_eq!(document.feature(sketch).unwrap().name, "Outline copy");
    let pasted = document.feature(solid).unwrap();
    assert_eq!(pasted.name, "Base copy");
    let Some(SolidFeature::Extrude(copy)) = pasted.kind.solid() else {
        panic!("an extrusion");
    };
    assert_eq!(copy.sketch, sketch);
    assert!(pasted.kind.uses_parameter(height));
    assert_eq!(document.features().len(), 4);
}

#[test]
fn features_from_another_model_take_parameters_by_name_or_their_values() {
    let Built {
        document: source,
        outline,
        base,
        ..
    } = built();
    let features = copied(&source, &[outline, base]);
    let parameters = carried(&source, &features);

    let mut target = Document::default();
    let paste = target
        .paste_features(&features, &parameters, PasteOrigin::Elsewhere)
        .unwrap();
    assert_eq!(paste.inlined, 1);
    target.apply(paste.transaction).unwrap();
    let solid = target.feature(paste.pasted[1]).unwrap();
    assert!(solid.kind.parameters().is_empty());
    let values = ParameterValues::evaluate(&target);
    let mut kind = solid.kind.clone();
    let heights: Vec<Quantity> = crate::inlining::expressions_mut(&mut kind)
        .into_iter()
        .map(|expression| values.evaluate_expression(expression).unwrap())
        .collect();
    assert_eq!(heights, vec![Quantity::length(4.0)]);

    let mut named = Document::default();
    let mut transaction = named.transaction("Height");
    let own = transaction.add_parameter("height", transaction.parse("9 mm").unwrap());
    named.apply(transaction.finish()).unwrap();
    let paste = named
        .paste_features(&features, &parameters, PasteOrigin::Elsewhere)
        .unwrap();
    assert_eq!(paste.inlined, 0);
    named.apply(paste.transaction).unwrap();
    assert!(
        named
            .feature(paste.pasted[1])
            .unwrap()
            .kind
            .uses_parameter(own)
    );
}

#[test]
fn references_outside_the_copy_are_kept_in_the_same_model_and_refused_elsewhere() {
    let Built {
        mut document,
        base,
        outline,
        ..
    } = built();
    let features = copied(&document, &[base]);
    let parameters = carried(&document, &features);

    let paste = document
        .paste_features(&features, &parameters, PasteOrigin::ThisDocument)
        .unwrap();
    document.apply(paste.transaction.clone()).unwrap();
    let Some(SolidFeature::Extrude(copy)) = document.feature(paste.pasted[0]).unwrap().kind.solid()
    else {
        panic!("an extrusion");
    };
    assert_eq!(copy.sketch, outline);

    let refused =
        Document::default().paste_features(&features, &parameters, PasteOrigin::Elsewhere);
    let Err(PasteError::NothingPasted { left_out }) = refused else {
        panic!("expected nothing pasted, got {refused:?}");
    };
    assert_eq!(left_out[0].name, "Base");
    assert_eq!(left_out[0].reason, PasteRefusal::OutsideFeature);
}

fn edge_at(solid: &Solid, point: Point3) -> EdgeReference {
    let (id, _) = solid
        .edges()
        .find(|(_, edge)| {
            let parameter = edge.curve().closest_parameter(point, edge.interval());
            edge.curve().point(parameter).distance(point) < 1e-6
        })
        .unwrap();
    EdgeReference::capture(solid, id).unwrap()
}

#[test]
fn a_feature_picking_edges_of_a_copied_body_is_left_out_with_its_reason() {
    let Built {
        mut document,
        outline,
        base,
        ..
    } = built();
    let evaluation = Recompute::default().run(
        &document,
        &ModelEvaluator,
        &CancelToken::never(),
        &|_, _| {},
    );
    let edges = vec![edge_at(
        evaluation.body(base).unwrap(),
        Point3::new(5.0, 0.0, 4.0),
    )];
    let mut transaction = document.transaction("Fillet");
    let fillet = transaction.add_feature(
        "Fillet 1",
        FeatureKind::Blend(Blend {
            kind: BlendKind::Fillet,
            body: base,
            edges,
            size: Expression::measure(1.0, caditor_expression::Unit::Millimetre),
            form: ChamferForm::Equal,
            flipped: false,
        }),
    );
    document.apply(transaction.finish()).unwrap();
    let features = copied(&document, &[outline, base, fillet]);

    let paste = document
        .paste_features(
            &features,
            &CarriedParameters::default(),
            PasteOrigin::ThisDocument,
        )
        .unwrap();

    assert_eq!(paste.pasted.len(), 2);
    assert_eq!(
        paste.left_out,
        vec![LeftOut {
            name: "Fillet 1".to_owned(),
            reason: PasteRefusal::CopiedGeometry {
                name: "Base".to_owned()
            },
        }]
    );
    let alone = document
        .paste_features(
            &features[2..],
            &CarriedParameters::default(),
            PasteOrigin::ThisDocument,
        )
        .unwrap();
    assert_eq!(alone.pasted.len(), 1);
}
