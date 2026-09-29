use std::collections::{BTreeMap, BTreeSet};

use caditor_geometry::Plane;

use super::*;
use crate::{
    build::{LinearExtent, extrude},
    profile::{PieceBound, Profile, ProfileCurve, Selection},
    test_support::{circle, rectangle},
    topology::Solid,
};

fn rectangle_piece() -> PieceId {
    PieceId::new(3, PieceBound::Start, PieceBound::End)
}

#[test]
fn digests_of_fixed_generators_never_change() {
    let side = FaceName::side(7, &rectangle_piece());
    assert_eq!(side.digest(), 0x3316_cc9f_3c6f_46ba_9c42_9387_328b_411b);
    let cut = PieceId::new(
        4,
        PieceBound::Cut {
            entities: vec![5, 9],
            occurrence: 1,
        },
        PieceBound::End,
    );
    assert_eq!(
        FaceName::side(7, &cut).digest(),
        0x0cf4_7a4b_3a10_d8aa_0cdd_735f_22df_e7a3
    );
    let start = FaceName::start_cap(7, RegionKey::from_digest(1));
    assert_eq!(start.digest(), 0x27b1_46a0_31d7_c4fe_1786_f8ea_d896_b7f3);
    assert_eq!(
        FaceName::end_cap(7, RegionKey::from_digest(1)).digest(),
        0x6471_512d_45e8_0858_40b9_22ea_479f_cb1c
    );
    assert_eq!(
        EdgeName::between(side, start).digest(),
        0xb689_39dc_9db5_8560_1d73_3a36_6a61_dd20
    );
    assert_eq!(
        EdgeName::seam(side).digest(),
        0x4bbf_e147_0017_a0d5_e399_304d_c2e0_00c3
    );
    let from = VertexName::of_faces([side, start]);
    assert_eq!(from.digest(), 0x9f5e_fad8_dc5a_d907_f299_5c9f_ada2_4a26);
    let to = VertexName::of_faces([side]);
    assert_eq!(
        EdgeName::between_at(side, start, from, to).digest(),
        0x982d_3cfb_6df7_fc08_c7b4_d7c5_b0dc_05f3
    );
    assert_eq!(
        EdgeName::occurrence(EdgeName::seam(side), 2).digest(),
        0x5309_f12b_64b1_f0ae_ecb3_6705_e055_2354
    );
    assert_eq!(
        FaceName::blend(7, EdgeName::seam(side)).digest(),
        0xc444_3fb2_438b_d42f_d809_707a_7f9a_16d4
    );
    assert_eq!(
        FaceName::corner(7, from).digest(),
        0x8288_425b_fc4f_dc2d_2748_c778_a522_373d
    );
    assert_eq!(
        FaceName::shell(7, side).digest(),
        0x9b1d_2291_bd8f_e7c6_4bcf_c06d_1486_7a90
    );
    assert_eq!(
        FaceName::imported(7, 3).digest(),
        0x3b1c_0e8d_e1cb_f80f_002b_d409_0740_b00a
    );
    let region = Profile::new(&rectangle(1, (0.0, 0.0), (4.0, 3.0))).unwrap();
    assert_eq!(
        region.regions()[0].key().digest(),
        0x39d2_066a_f1ef_e5d7_8c1c_545d_7cec_6813
    );
}

#[test]
fn names_are_canonical_in_their_unordered_inputs() {
    let a = FaceName::side(1, &rectangle_piece());
    let b = FaceName::start_cap(1, RegionKey::from_digest(5));
    let c = FaceName::end_cap(1, RegionKey::from_digest(5));
    assert_eq!(EdgeName::between(a, b), EdgeName::between(b, a));
    assert_eq!(
        VertexName::of_faces([a, b, c]),
        VertexName::of_faces([c, a, b, a])
    );
    let (from, to) = (VertexName::of_faces([a, b]), VertexName::of_faces([a, c]));
    assert_eq!(
        EdgeName::between_at(a, b, from, to),
        EdgeName::between_at(b, a, to, from)
    );
    assert_ne!(
        EdgeName::between_at(a, b, from, to),
        EdgeName::between_at(b, a, from, to)
    );
    assert_eq!(
        EdgeName::between_at(a, b, from, from),
        EdgeName::between_at(b, a, from, from)
    );
    assert_ne!(
        EdgeName::between_at(a, b, from, from),
        EdgeName::between_at(a, c, from, from)
    );
}

#[test]
fn distinct_generators_give_distinct_names() {
    let piece = rectangle_piece();
    let key = RegionKey::from_digest(5);
    let faces = [
        FaceName::side(1, &piece),
        FaceName::side(2, &piece),
        FaceName::side(1, &PieceId::new(3, PieceBound::End, PieceBound::Start)),
        FaceName::side(1, &PieceId::new(4, PieceBound::Start, PieceBound::End)),
        FaceName::start_cap(1, key),
        FaceName::end_cap(1, key),
        FaceName::start_cap(2, key),
        FaceName::start_cap(1, RegionKey::from_digest(6)),
    ];
    let distinct: BTreeSet<FaceName> = faces.iter().copied().collect();
    assert_eq!(distinct.len(), faces.len());
    let edges = [
        EdgeName::between(faces[0], faces[4]),
        EdgeName::between(faces[0], faces[5]),
        EdgeName::seam(faces[0]),
        EdgeName::between(faces[0], faces[0]),
        EdgeName::occurrence(EdgeName::seam(faces[0]), 0),
        EdgeName::occurrence(EdgeName::seam(faces[0]), 1),
    ];
    let distinct: BTreeSet<EdgeName> = edges.iter().copied().collect();
    assert_eq!(distinct.len(), edges.len());
    assert!(FaceName::default().is_none());
    assert_eq!(FaceName::from_digest(9).digest(), 9);
}

fn named_faces(solid: &Solid) -> BTreeMap<FaceName, FaceOrigin> {
    solid
        .faces()
        .map(|(_, face)| (face.name(), face.origin().unwrap()))
        .collect()
}

fn plate(width: f64, hole: (f64, f64)) -> Solid {
    let mut curves: Vec<ProfileCurve> = rectangle(1, (0.0, 0.0), (width, 8.0));
    curves.push(circle(5, hole, 1.5));
    let regions = Profile::new(&curves)
        .unwrap()
        .select(&Selection::EvenDepth)
        .unwrap();
    extrude(
        &Plane::XY,
        &regions,
        LinearExtent::one_side(2.0).unwrap(),
        11,
    )
    .unwrap()
}

#[test]
fn moving_a_hole_keeps_every_face_name() {
    let before = named_faces(&plate(10.0, (4.0, 4.0)));
    let after = named_faces(&plate(10.0, (6.0, 3.0)));
    assert_eq!(before, after);
    assert_eq!(before.len(), 7);
    let origins: BTreeSet<FaceOrigin> = before.values().copied().collect();
    assert!(origins.contains(&FaceOrigin::Side {
        feature: 11,
        entity: 5
    }));
    assert!(origins.contains(&FaceOrigin::StartCap { feature: 11 }));
    assert!(origins.contains(&FaceOrigin::EndCap { feature: 11 }));
}

#[test]
fn resizing_the_outline_keeps_the_hole_name() {
    let hole = |solid: &Solid| {
        solid
            .faces()
            .find(|(_, face)| face.origin().and_then(|origin| origin.entity()) == Some(5))
            .map(|(_, face)| face.name())
            .unwrap()
    };
    let small = plate(10.0, (4.0, 4.0));
    let large = plate(14.0, (4.0, 4.0));
    assert_eq!(hole(&small), hole(&large));
    assert_eq!(named_faces(&small), named_faces(&large));
    let edges = |solid: &Solid| -> BTreeSet<EdgeName> {
        solid.edges().map(|(_, edge)| edge.name()).collect()
    };
    assert_eq!(edges(&small), edges(&large));
}

#[test]
fn features_and_directions_name_faces_apart() {
    let regions = Profile::new(&rectangle(1, (0.0, 0.0), (4.0, 3.0)))
        .unwrap()
        .select(&Selection::EvenDepth)
        .unwrap();
    let up = extrude(
        &Plane::XY,
        &regions,
        LinearExtent::one_side(2.0).unwrap(),
        1,
    )
    .unwrap();
    let down = extrude(
        &Plane::XY,
        &regions,
        LinearExtent::one_side(-2.0).unwrap(),
        1,
    )
    .unwrap();
    let other = extrude(
        &Plane::XY,
        &regions,
        LinearExtent::one_side(2.0).unwrap(),
        2,
    )
    .unwrap();
    assert_eq!(named_faces(&up), named_faces(&down));
    let shared: BTreeSet<FaceName> = named_faces(&up)
        .keys()
        .copied()
        .filter(|name| named_faces(&other).contains_key(name))
        .collect();
    assert!(shared.is_empty());
    let sketch_plane_cap = |solid: &Solid| {
        solid
            .faces()
            .find(|(_, face)| face.origin() == Some(FaceOrigin::StartCap { feature: 1 }))
            .map(|(_, face)| face.surface().clone())
            .unwrap()
    };
    let height = |surface: crate::surface::Surface| match surface {
        crate::surface::Surface::Plane(plane) => plane.frame().origin().z,
        _ => f64::NAN,
    };
    assert_eq!(height(sketch_plane_cap(&up)), 0.0);
    assert_eq!(height(sketch_plane_cap(&down)), 0.0);
}
