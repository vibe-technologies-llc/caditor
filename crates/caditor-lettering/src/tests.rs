use caditor_geometry::Point2;
use caditor_kernel::{Profile, ProfileCurve};

use crate::{Alignment, Lettered, Segment, Style, lettered};

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
