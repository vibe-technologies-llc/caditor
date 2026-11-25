use crate::*;

fn fit(code: &str) -> IsoFit {
    IsoFit::from_code(code).unwrap()
}

fn in_tenth_micrometres(code: &str, size: f64) -> (i64, i64) {
    let (upper, lower) = fit(code).deviations(size).unwrap();
    (
        (upper * 10_000.0).round() as i64,
        (lower * 10_000.0).round() as i64,
    )
}

fn tenths(upper: f64, lower: f64) -> (i64, i64) {
    ((upper * 10.0).round() as i64, (lower * 10.0).round() as i64)
}

#[test]
fn hole_and_shaft_deviations_match_the_published_iso_286_2_tables() {
    let published = [
        ("H7", 10.0, (15.0, 0.0)),
        ("H7", 25.0, (21.0, 0.0)),
        ("H8", 40.0, (39.0, 0.0)),
        ("H11", 3.0, (60.0, 0.0)),
        ("H7", 500.0, (63.0, 0.0)),
        ("h6", 25.0, (0.0, -13.0)),
        ("h9", 100.0, (0.0, -87.0)),
        ("g6", 25.0, (-7.0, -20.0)),
        ("g6", 8.0, (-5.0, -14.0)),
        ("f7", 25.0, (-20.0, -41.0)),
        ("f7", 2.0, (-6.0, -16.0)),
        ("f8", 160.0, (-43.0, -106.0)),
        ("js6", 25.0, (6.5, -6.5)),
        ("js7", 25.0, (10.0, -10.0)),
        ("js9", 15.0, (21.0, -21.0)),
        ("js5", 2.0, (2.0, -2.0)),
        ("k6", 25.0, (15.0, 2.0)),
        ("k6", 450.0, (45.0, 5.0)),
        ("k8", 25.0, (33.0, 0.0)),
        ("k7", 2.0, (10.0, 0.0)),
        ("n6", 25.0, (28.0, 15.0)),
        ("n6", 60.0, (39.0, 20.0)),
        ("p6", 25.0, (35.0, 22.0)),
        ("p6", 300.0, (88.0, 56.0)),
        ("p7", 5.0, (24.0, 12.0)),
    ];

    for (code, size, (upper, lower)) in published {
        assert_eq!(
            in_tenth_micrometres(code, size),
            tenths(upper, lower),
            "{code} at {size} mm"
        );
    }
}

#[test]
fn a_size_on_a_step_belongs_to_the_range_it_ends_and_rounding_noise_keeps_it_there() {
    assert_eq!(in_tenth_micrometres("H7", 10.0), (150, 0));
    assert_eq!(in_tenth_micrometres("H7", 10.000_000_000_1), (150, 0));
    assert_eq!(in_tenth_micrometres("H7", 10.001), (180, 0));
}

#[test]
fn sizes_outside_the_table_and_unknown_codes_have_no_deviations() {
    assert_eq!(fit("H7").deviations(0.0), None);
    assert_eq!(fit("H7").deviations(-4.0), None);
    assert_eq!(fit("H7").deviations(500.5), None);
    assert_eq!(fit("H7").deviations(f64::NAN), None);

    assert_eq!(IsoFit::from_code("H4"), None);
    assert_eq!(IsoFit::from_code("H12"), None);
    assert_eq!(IsoFit::from_code("K7"), None);
    assert_eq!(IsoFit::from_code("H"), None);
    assert_eq!(IsoFit::from_code("7"), None);
}

#[test]
fn every_letter_and_grade_reads_back_from_its_code() {
    for letter in FitLetter::ALL {
        for grade in FIT_GRADES {
            let made = IsoFit::new(letter, grade).unwrap();

            assert_eq!(IsoFit::from_code(&made.code()), Some(made));
            assert!(made.deviations(25.0).is_some());
        }
    }
}
