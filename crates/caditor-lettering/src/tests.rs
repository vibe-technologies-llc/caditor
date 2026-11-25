use caditor_geometry::Point2;
use caditor_kernel::{Profile, ProfileCurve, interruptible};

use crate::{Alignment, INTER, INTER_ITALIC, Lettered, Segment, Style, Typeface, lettered};

const REGULAR: Style = Style {
    bold: false,
    italic: false,
};
const CLOSE: f64 = 1e-6;

fn bounds(lettered: &Lettered) -> (Point2, Point2) {
    lettered
        .segments()
        .flat_map(|segment| segment.sampled(8))
        .fold(
            (
                Point2::splat(f64::INFINITY),
                Point2::splat(f64::NEG_INFINITY),
            ),
            |(low, high), point| (low.min(point), high.max(point)),
        )
}

fn profile(lettered: &Lettered) -> Profile {
    let curves: Vec<ProfileCurve> = lettered
        .segments()
        .zip(0_u64..)
        .map(|(segment, entity)| segment.profile_curve(entity))
        .collect();
    Profile::new(&curves).unwrap()
}

fn ink(lettered: &Lettered) -> f64 {
    profile(lettered)
        .regions()
        .iter()
        .filter(|region| region.depth() % 2 == 0)
        .filter_map(|region| region.section())
        .map(|section| section.area)
        .sum()
}

#[test]
fn a_capital_stands_one_cap_height_tall_on_the_baseline() {
    let letters = lettered("H", REGULAR, Alignment::Left).unwrap();

    let (low, high) = bounds(&letters);

    assert!(low.y.abs() < CLOSE, "{low:?}");
    assert!((high.y - 1.0).abs() < CLOSE, "{high:?}");
    assert!(low.x >= 0.0);
    assert!(letters.width > high.x);
}

#[test]
fn letters_with_counters_become_an_outline_and_holes() {
    let letters = lettered("oB", REGULAR, Alignment::Left).unwrap();

    let regions = profile(&letters).regions().to_vec();
    let solid = regions.iter().filter(|region| region.depth() == 0).count();
    let holes = regions.iter().filter(|region| region.depth() == 1).count();

    assert_eq!(solid, 2);
    assert_eq!(holes, 3);
}

#[test]
fn overlapping_contours_of_a_letter_are_merged_into_one_outline() {
    for character in ['e', 't', 'f', 'k', 'A'] {
        let letters = lettered(&character.to_string(), REGULAR, Alignment::Left).unwrap();

        let regions = profile(&letters).regions().to_vec();

        assert!(
            regions.iter().all(|region| region.depth() <= 1),
            "{character}: {:?}",
            regions
                .iter()
                .map(|region| region.depth())
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn kerning_pulls_a_pair_closer_than_its_letters_apart() {
    let pair = lettered("AV", REGULAR, Alignment::Left).unwrap().width;
    let apart = lettered("A", REGULAR, Alignment::Left).unwrap().width
        + lettered("V", REGULAR, Alignment::Left).unwrap().width;

    assert!(pair < apart - CLOSE, "{pair} against {apart}");
}

#[test]
fn bold_letters_carry_more_ink_and_italic_ones_lean() {
    let regular = lettered("l", REGULAR, Alignment::Left).unwrap();
    let bold = lettered(
        "l",
        Style {
            bold: true,
            ..REGULAR
        },
        Alignment::Left,
    )
    .unwrap();
    let italic = lettered(
        "l",
        Style {
            italic: true,
            ..REGULAR
        },
        Alignment::Left,
    )
    .unwrap();

    let foot = |letters: &Lettered| {
        letters
            .segments()
            .flat_map(|segment| segment.sampled(8))
            .filter(|point| point.y < 0.1)
            .map(|point| point.x)
            .fold(f64::INFINITY, f64::min)
    };
    let head = |letters: &Lettered| {
        letters
            .segments()
            .flat_map(|segment| segment.sampled(8))
            .filter(|point| point.y > 0.9)
            .map(|point| point.x)
            .fold(f64::INFINITY, f64::min)
    };

    assert!(ink(&bold) > 1.3 * ink(&regular));
    assert!(head(&italic) - foot(&italic) > 0.1);
    assert!((head(&regular) - foot(&regular)).abs() < CLOSE);
}

#[test]
fn spaces_advance_and_characters_inter_lacks_are_named() {
    let spaced = lettered("a b", REGULAR, Alignment::Left).unwrap();
    let missing = lettered("a\u{4e2d}b", REGULAR, Alignment::Left).unwrap();

    assert_eq!(spaced.letters.len(), 2);
    assert!(spaced.width > lettered("ab", REGULAR, Alignment::Left).unwrap().width);
    assert_eq!(missing.missing, vec!['\u{4e2d}']);
    assert_eq!(
        missing
            .letters
            .iter()
            .map(|letter| letter.position)
            .collect::<Vec<_>>(),
        vec![0, 2]
    );
}

#[test]
fn every_letter_outline_closes() {
    let letters = lettered("Caditor 2026 &?", REGULAR, Alignment::Left).unwrap();

    for letter in &letters.letters {
        let ends = |segment: &Segment| match segment {
            Segment::Line(start, end) => (*start, *end),
            Segment::Bezier(points) => (
                points.first().copied().unwrap_or_default(),
                points.last().copied().unwrap_or_default(),
            ),
        };
        for segment in &letter.segments {
            let (_, end) = ends(segment);

            assert!(
                letter
                    .segments
                    .iter()
                    .any(|other| ends(other).0.distance(end) < CLOSE),
                "{}",
                letter.character
            );
        }
    }
}

fn left(content: &str) -> Lettered {
    lettered(content, REGULAR, Alignment::Left).unwrap()
}

fn lowest_and_highest(letters: &Lettered, position: usize) -> (f64, f64) {
    let (low, high) = bounds(&Lettered {
        letters: letters
            .letters
            .iter()
            .filter(|letter| letter.position == position)
            .cloned()
            .collect(),
        ..Lettered::default()
    });
    (low.y, high.y)
}

#[test]
fn a_line_break_sets_the_next_line_one_pitch_below() {
    let two = left("H\nH");

    let (first_low, first_high) = lowest_and_highest(&two, 0);
    let (second_low, second_high) = lowest_and_highest(&two, 2);

    assert!(
        two.line_pitch > 1.2 && two.line_pitch < 2.2,
        "{}",
        two.line_pitch
    );
    assert!((first_low - 0.0).abs() < CLOSE && (first_high - 1.0).abs() < CLOSE);
    assert!((second_low + two.line_pitch).abs() < CLOSE);
    assert!((second_high - (1.0 - two.line_pitch)).abs() < CLOSE);
    assert_eq!(two.line_widths.len(), 2);
}

#[test]
fn every_line_starts_at_the_anchor_and_the_widest_sets_the_width() {
    let lines = left("ab\nabcd\n\nc");

    assert_eq!(lines.line_widths.len(), 4);
    assert!((lines.line_widths[0] - left("ab").width).abs() < CLOSE);
    assert!((lines.line_widths[1] - left("abcd").width).abs() < CLOSE);
    assert_eq!(lines.line_widths[2], 0.0);
    assert_eq!(lines.width, lines.line_widths[1]);
    assert_eq!(
        lines
            .letters
            .iter()
            .map(|letter| letter.position)
            .collect::<Vec<_>>(),
        vec![0, 1, 3, 4, 5, 6, 9]
    );

    let (low, _) = bounds(&Lettered {
        letters: lines
            .letters
            .iter()
            .filter(|l| l.position == 3)
            .cloned()
            .collect(),
        ..Lettered::default()
    });

    assert!(low.x >= 0.0 && low.x < 0.2);
}

#[test]
fn centred_and_right_lines_hang_about_the_anchor_by_their_own_widths() {
    let content = "ab\nabcd";
    let plain = left(content);
    let centred = lettered(content, REGULAR, Alignment::Centre).unwrap();
    let right = lettered(content, REGULAR, Alignment::Right).unwrap();

    let first_x = |letters: &Lettered, position: usize| {
        letters
            .letters
            .iter()
            .find(|letter| letter.position == position)
            .and_then(|letter| letter.segments.first())
            .map(|segment| segment.sampled(1)[0].x)
            .unwrap()
    };

    for (position, width) in [(0, plain.line_widths[0]), (3, plain.line_widths[1])] {
        let origin = first_x(&plain, position);

        assert!((first_x(&centred, position) - (origin - width / 2.0)).abs() < CLOSE);
        assert!((first_x(&right, position) - (origin - width)).abs() < CLOSE);
    }

    assert_eq!(centred.line_widths, plain.line_widths);
    assert_eq!(right.width, plain.width);
}

fn inter() -> Typeface<'static> {
    Typeface::parse(INTER, Some(INTER_ITALIC)).unwrap()
}

#[test]
fn every_whole_weight_from_101_to_899_shares_nine_cached_outlines() {
    let mut typeface = inter();

    let glyph = typeface
        .glyph('e', typeface.instance(400.0, false))
        .unwrap();

    for weight in 101..=899 {
        let instance = typeface.instance(f64::from(weight), false);

        typeface.outline(instance, glyph).unwrap();
    }

    assert!(
        typeface.cached_outlines() <= 9,
        "{}",
        typeface.cached_outlines()
    );
    assert!(typeface.cached_outlines() > 1);
}

#[test]
fn a_weight_is_drawn_at_the_nearest_hundred_inside_the_face() {
    let typeface = inter();

    assert_eq!(
        typeface.instance(449.0, false),
        typeface.instance(400.0, false)
    );
    assert_eq!(
        typeface.instance(451.0, false),
        typeface.instance(500.0, false)
    );
    assert_eq!(
        typeface.instance(1.0, false),
        typeface.instance(100.0, false)
    );
    assert_eq!(
        typeface.instance(5000.0, true),
        typeface.instance(900.0, true)
    );
}

#[test]
fn a_cancelled_merge_leaves_nothing_cached_and_the_next_one_succeeds() {
    let mut typeface = inter();

    let instance = typeface.instance(400.0, false);
    let glyph = typeface.glyph('e', instance).unwrap();

    let cancelled = interruptible(std::sync::Arc::new(|| true), || {
        typeface.outline(instance, glyph)
    });

    assert!(cancelled.is_err());
    assert_eq!(typeface.cached_outlines(), 0);
    assert!(typeface.outline(instance, glyph).is_ok());
    assert_eq!(typeface.cached_outlines(), 1);
}

const SIGNS: [char; 4] = ['\u{2300}', '\u{21a7}', '\u{2334}', '\u{2335}'];

fn horizontal_range(letters: &Lettered, position: usize) -> (f64, f64) {
    let (low, high) = bounds(&Lettered {
        letters: letters
            .letters
            .iter()
            .filter(|letter| letter.position == position)
            .cloned()
            .collect(),
        ..Lettered::default()
    });
    (low.x, high.x)
}

#[test]
fn the_diameter_sign_is_drawn_and_not_reported_missing() {
    let letters = left("\u{2300}20");

    assert!(letters.missing.is_empty());
    assert_eq!(
        letters
            .letters
            .iter()
            .map(|letter| letter.character)
            .collect::<Vec<_>>(),
        vec!['\u{2300}', '2', '0']
    );
    assert_eq!(
        letters
            .letters
            .iter()
            .map(|letter| letter.position)
            .collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
}

#[test]
fn the_diameter_sign_is_a_ring_split_by_its_slash_and_clears_its_digits() {
    let letters = left("\u{2300}20");

    let regions = profile(&left("\u{2300}")).regions().to_vec();
    let solid = regions.iter().filter(|region| region.depth() == 0).count();
    let holes = regions.iter().filter(|region| region.depth() == 1).count();
    let (sign_left, sign_right) = horizontal_range(&letters, 0);
    let (digit_left, _) = horizontal_range(&letters, 1);
    let (low, high) = lowest_and_highest(&letters, 0);

    assert_eq!(solid, 1);
    assert_eq!(holes, 2);
    assert!(sign_left > 0.0);
    assert!(digit_left - sign_right > 0.02, "{sign_right} {digit_left}");
    assert!(digit_left - sign_right < 0.3, "{sign_right} {digit_left}");
    assert!((-0.1..0.0).contains(&low), "{low}");
    assert!((1.0..1.1).contains(&high), "{high}");
    assert!(letters.width > lettered("20", REGULAR, Alignment::Left).unwrap().width + 1.0);
}

#[test]
fn every_technical_sign_stays_in_its_advance_and_draws_one_closed_outline() {
    for character in SIGNS {
        for style in [
            REGULAR,
            Style {
                bold: true,
                italic: false,
            },
        ] {
            let letters = lettered(&character.to_string(), style, Alignment::Left).unwrap();

            let (low, high) = bounds(&letters);
            let regions = profile(&letters).regions().to_vec();

            assert!(letters.missing.is_empty(), "{character}");
            assert!(low.x >= 0.0 && high.x <= letters.width, "{character}");
            assert!(low.y >= -0.1 && high.y <= 1.1, "{character}");
            assert!(
                regions.iter().any(|region| region.depth() == 0),
                "{character}"
            );
            assert!(ink(&letters) > 0.05, "{character}");
        }
    }
}

#[test]
fn the_technical_signs_follow_the_weight_and_lean_with_italic() {
    for character in SIGNS {
        let text = character.to_string();
        let regular = lettered(&text, REGULAR, Alignment::Left).unwrap();
        let bold = lettered(
            &text,
            Style {
                bold: true,
                italic: false,
            },
            Alignment::Left,
        )
        .unwrap();
        let italic = lettered(
            &text,
            Style {
                bold: false,
                italic: true,
            },
            Alignment::Left,
        )
        .unwrap();

        assert!(ink(&bold) > 1.2 * ink(&regular), "{character}");
        assert!(bounds(&italic).1.x > bounds(&regular).1.x, "{character}");
    }
}

#[test]
fn a_sign_between_letters_takes_no_kerning_and_keeps_the_neighbours_apart() {
    let apart = left("M\u{2300}M");

    let (_, first_right) = horizontal_range(&apart, 0);
    let (sign_left, sign_right) = horizontal_range(&apart, 1);
    let (second_left, _) = horizontal_range(&apart, 2);

    assert!(sign_left > first_right);
    assert!(second_left > sign_right);
}
