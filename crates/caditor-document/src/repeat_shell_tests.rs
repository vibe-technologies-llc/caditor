use std::collections::BTreeSet;

use caditor_expression::{Expression, Unit};
use caditor_kernel::{FaceOrigin, Made, Solid};

use crate::{
    pattern_tests::failure,
    repeat_blend_tests::{
        Boss, PLATE, assert_close, evaluate, exact_volume, face_made_by, linear, square_boss,
        volume_after,
    },
    *,
};

const BOSS: f64 = 4.0 * 4.0 * 2.0;
const BOSS_HOLLOW: f64 = 3.0 * 3.0 * 2.5;

struct Hollowed {
    boss: Boss,
    shell: FeatureId,
}

fn hollowed_boss() -> Hollowed {
    let mut boss = square_boss();
    let evaluation = evaluate(&boss.document);
    let top = face_made_by(
        evaluation.body(boss.plate).unwrap(),
        FaceOrigin::EndCap {
            feature: boss.boss.raw(),
        },
    );
    let mut transaction = boss.document.transaction("Shell");
    let shell = transaction.add_feature(
        "Shell 1",
        FeatureKind::Shell(Shell {
            body: boss.plate,
            open: vec![top],
            thickness: Expression::measure(0.5, Unit::Millimetre),
            side: ShellSide::Inside,
            faces: Vec::new(),
        }),
    );
    boss.document.apply(transaction.finish()).unwrap();
    Hollowed { boss, shell }
}

fn add(document: &mut Document, name: &str, kind: FeatureKind) -> FeatureId {
    let mut transaction = document.transaction(name);
    let feature = transaction.add_feature(name, kind);
    document.apply(transaction.finish()).unwrap();
    feature
}

fn copies_made(
    solid: &Solid,
    repeating: FeatureId,
    feature: FeatureId,
    made: Made,
) -> BTreeSet<[u32; 2]> {
    solid
        .faces()
        .filter_map(|(_, face)| face.origin())
        .filter_map(|origin| match origin {
            FaceOrigin::Copy {
                copy,
                feature: from,
                made: kind,
            } if copy.pattern == repeating.raw() && from == feature.raw() && kind == made => {
                Some(copy.index)
            }
            _ => None,
        })
        .collect()
}

#[test]
fn a_pattern_of_a_hollowed_boss_opens_and_hollows_every_copy() {
    let Hollowed { mut boss, shell } = hollowed_boss();
    let pattern = Pattern::new(boss.plate, linear(boss.count)).repeating(vec![boss.boss, shell]);
    let pattern = add(&mut boss.document, "Pattern 1", FeatureKind::from(pattern));
    let evaluation = evaluate(&boss.document);

    assert_eq!(evaluation.failed_count(), 0);
    let hollowed = volume_after(&evaluation, shell);
    assert!(hollowed < PLATE / 2.0);
    assert_close(
        exact_volume(&evaluation, boss.plate),
        hollowed + 2.0 * (BOSS - BOSS_HOLLOW),
    );
    let solid = evaluation.body(boss.plate).unwrap();
    assert_eq!(
        copies_made(solid, pattern, shell, Made::Shell),
        BTreeSet::from([[1, 0], [2, 0]])
    );
    let opened_tops = solid
        .faces()
        .map(|(_, face)| face)
        .filter(|face| {
            face.origin().is_some_and(|origin| {
                origin
                    .copy()
                    .is_some_and(|copy| copy.pattern == pattern.raw())
                    && origin.original()
                        == FaceOrigin::EndCap {
                            feature: boss.boss.raw(),
                        }
            })
        })
        .filter(|face| face.loops().len() == 2)
        .count();
    assert_eq!(opened_tops, 2);
}

#[test]
fn a_mirror_of_a_hollowed_boss_hollows_its_image() {
    let Hollowed { mut boss, shell } = hollowed_boss();
    let mirror = Mirror::new(boss.plate, PlaneReference::Principal(PrincipalPlane::Yz))
        .mirroring(vec![boss.boss, shell]);
    let mirror = add(&mut boss.document, "Mirror 1", FeatureKind::Mirror(mirror));
    let evaluation = evaluate(&boss.document);

    assert_eq!(evaluation.failed_count(), 0);
    assert_close(
        exact_volume(&evaluation, boss.plate),
        volume_after(&evaluation, shell) + BOSS - BOSS_HOLLOW,
    );
    let solid = evaluation.body(boss.plate).unwrap();
    assert_eq!(
        copies_made(solid, mirror, shell, Made::Shell),
        BTreeSet::from([MIRROR_IMAGE])
    );
}

#[test]
fn a_shell_followed_by_another_change_to_its_body_is_refused_in_words() {
    let Hollowed { mut boss, shell } = hollowed_boss();
    let mm = |value: f64| Expression::measure(value, Unit::Millimetre);
    add(
        &mut boss.document,
        "Peg",
        FeatureKind::Primitive(Primitive {
            shape: PrimitiveShape::Cylinder {
                diameter: mm(2.0),
                height: mm(3.0),
            },
            plane: PlaneReference::Principal(PrincipalPlane::Xy),
            at: [mm(10.0), mm(10.0)],
            anchor: PrimitiveAnchor::BaseCentre,
            reversed: false,
            operation: BodyOperation::Add(boss.plate),
        }),
    );
    let pattern = Pattern::new(boss.plate, linear(boss.count)).repeating(vec![boss.boss, shell]);
    let pattern = add(&mut boss.document, "Pattern 1", FeatureKind::from(pattern));
    let evaluation = evaluate(&boss.document);

    let error = failure(&evaluation, pattern);
    assert_eq!(
        error.reason,
        "Peg changes the body after Shell 1, so this pattern cannot hollow its copies with Shell 1."
    );
    assert_eq!(
        error.remedy,
        "Move this pattern right below Shell 1, or leave Shell 1 out of it."
    );
    assert_eq!(
        repeatable_on(&boss.document.feature(shell).unwrap().kind),
        Some(boss.plate)
    );
}
