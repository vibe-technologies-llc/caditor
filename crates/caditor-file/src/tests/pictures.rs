use std::{io::Cursor, sync::Arc};

use caditor_document::{
    Calibration, DEFAULT_IMAGE_OPACITY_PERCENT, Document, Edit, Editor, FeatureId, FeatureKind,
    ImagePlane, ImageSize, ImageSpot, Picture, PictureFormat, PlaneReference, PrincipalPlane,
    ReferenceImage, Transaction,
};
use caditor_expression::{Expression, Unit};
use image::{
    Delay, DynamicImage, Frame, ImageBuffer, ImageFormat, Rgba, RgbaImage, codecs::gif::GifEncoder,
};
use tempfile::TempDir;

use super::{config, crash, decode_lines, lines_of, untitled};
use crate::{JournalEntry, PictureReadError, Storage, decode_picture, journal, read_picture, scan};

const WIDTH: u32 = 8;
const HEIGHT: u32 = 6;
const RED: [u8; 3] = [255, 0, 0];
const BLUE: [u8; 3] = [0, 0, 255];
const GREEN: [u8; 3] = [0, 255, 0];
const EXACT: u8 = 0;
const LOSSY: u8 = 70;
const JPEG_XL: &[u8] = include_bytes!("../../pictures/quad.jxl");
const JPEG_XL_CONTAINER: &[u8] = include_bytes!("../../pictures/quad-container.jxl");
const LOSSY_WEBP: &[u8] = include_bytes!("../../pictures/quad-lossy.webp");

fn quad_colour(x: u32, y: u32) -> [u8; 3] {
    match (x >= WIDTH / 2, y >= HEIGHT / 2) {
        (true, false) => BLUE,
        (false, true) => GREEN,
        (false, false) | (true, true) => RED,
    }
}

fn quad() -> RgbaImage {
    ImageBuffer::from_fn(WIDTH, HEIGHT, |x, y| {
        let [red, green, blue] = quad_colour(x, y);
        Rgba([red, green, blue, 255])
    })
}

fn encoded(image: &DynamicImage, format: ImageFormat) -> Vec<u8> {
    let mut bytes = Cursor::new(Vec::new());
    image.write_to(&mut bytes, format).unwrap();
    bytes.into_inner()
}

fn assert_quad(bytes: &[u8], tolerance: u8, what: &str) {
    let pixels = decode_picture(bytes).unwrap_or_else(|error| panic!("{what}: {error}"));
    assert_eq!((pixels.width, pixels.height), (WIDTH, HEIGHT), "{what}");
    for (x, y) in [(1, 1), (6, 1), (1, 4), (6, 4)] {
        let at = ((y * WIDTH + x) * 4) as usize;
        let read = &pixels.rgba[at..at + 3];
        let near = read
            .iter()
            .zip(quad_colour(x, y))
            .all(|(read, wanted)| read.abs_diff(wanted) <= tolerance);
        assert!(near, "{what} at {x}, {y} read {read:?}");
    }
}

#[test]
fn every_picture_format_caditor_reads_decodes_to_its_pixels() {
    let image = DynamicImage::ImageRgba8(quad());
    let exact = [
        ImageFormat::Png,
        ImageFormat::WebP,
        ImageFormat::Gif,
        ImageFormat::Bmp,
        ImageFormat::Tiff,
        ImageFormat::Qoi,
        ImageFormat::Tga,
    ];
    for format in exact {
        assert_quad(&encoded(&image, format), EXACT, &format!("{format:?}"));
    }
    let opaque = DynamicImage::ImageRgb8(image.to_rgb8());
    assert_quad(&encoded(&opaque, ImageFormat::Jpeg), LOSSY, "JPEG");
    let deep = DynamicImage::ImageRgba16(image.to_rgba16());
    assert_quad(&encoded(&deep, ImageFormat::Tiff), EXACT, "16-bit TIFF");
    assert_quad(JPEG_XL, EXACT, "JPEG XL");
    assert_quad(JPEG_XL_CONTAINER, EXACT, "JPEG XL in its container");
    assert_quad(LOSSY_WEBP, LOSSY, "lossy WebP");

    let formats: Vec<PictureFormat> = [
        encoded(&image, ImageFormat::Png),
        encoded(&opaque, ImageFormat::Jpeg),
        LOSSY_WEBP.to_vec(),
        JPEG_XL.to_vec(),
        encoded(&image, ImageFormat::Gif),
        encoded(&image, ImageFormat::Bmp),
        encoded(&image, ImageFormat::Tiff),
        encoded(&image, ImageFormat::Qoi),
        encoded(&image, ImageFormat::Tga),
    ]
    .into_iter()
    .map(|bytes| read_picture(bytes).unwrap().picture.format())
    .collect();
    assert_eq!(formats, PictureFormat::ALL.to_vec());
}

#[test]
fn a_gif_reads_its_first_frame_and_a_tiff_its_first_page() {
    let blank = RgbaImage::from_pixel(WIDTH, HEIGHT, Rgba([255, 255, 255, 255]));
    let mut gif = Vec::new();
    {
        let mut encoder = GifEncoder::new(&mut gif);
        encoder
            .encode_frames([
                Frame::from_parts(quad(), 0, 0, Delay::from_numer_denom_ms(100, 1)),
                Frame::from_parts(blank.clone(), 0, 0, Delay::from_numer_denom_ms(100, 1)),
            ])
            .unwrap();
    }
    assert_quad(&gif, EXACT, "animated GIF");

    let mut tiff = Cursor::new(Vec::new());
    {
        let mut encoder = tiff::encoder::TiffEncoder::new(&mut tiff).unwrap();
        encoder
            .write_image::<tiff::encoder::colortype::RGBA8>(WIDTH, HEIGHT, quad().as_raw())
            .unwrap();
        encoder
            .write_image::<tiff::encoder::colortype::RGBA8>(WIDTH, HEIGHT, blank.as_raw())
            .unwrap();
    }
    assert_quad(&tiff.into_inner(), EXACT, "two-page TIFF");
}

fn png_chunk(kind: &[u8], data: &[u8]) -> Vec<u8> {
    let mut checked = kind.to_vec();
    checked.extend_from_slice(data);
    let mut chunk = u32::try_from(data.len()).unwrap().to_be_bytes().to_vec();
    chunk.extend_from_slice(&checked);
    chunk.extend_from_slice(&crc32fast::hash(&checked).to_be_bytes());
    chunk
}

fn png_claiming(width: u32, height: u32) -> Vec<u8> {
    let mut header = width.to_be_bytes().to_vec();
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, 6, 0, 0, 0]);
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    bytes.extend(png_chunk(b"IHDR", &header));
    bytes.extend(png_chunk(
        b"IDAT",
        &[0x78, 0x9c, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01],
    ));
    bytes.extend(png_chunk(b"IEND", &[]));
    bytes
}

#[test]
fn hostile_damaged_or_unreadable_pictures_are_refused_in_words_before_decoding() {
    let words = |bytes: &[u8]| decode_picture(bytes).unwrap_err().to_string();

    assert_eq!(
        words(&png_claiming(100_000, 100_000)),
        "it is 100000 by 100000 pixels, more than 16384 on a side"
    );
    assert_eq!(
        words(&png_claiming(10_000, 10_000)),
        "it is 10000 by 10000 pixels, more than 40 million in all"
    );
    assert_eq!(
        words(b"\0\0\0\x1cftypavif\0\0\0\0mif1"),
        "it is an AVIF picture, which caditor cannot read yet; save it as PNG or JPEG"
    );
    assert_eq!(
        words(b"\0\0\0\x18ftypheic\0\0\0\0mif1"),
        "it is an HEIC picture, which caditor cannot read yet; save it as PNG or JPEG"
    );
    assert_eq!(words(b"plain text"), "it is not a picture caditor can read");
    assert_eq!(words(b""), "it is empty");
    let png = encoded(&DynamicImage::ImageRgba8(quad()), ImageFormat::Png);
    for cut in [png.len() / 2, png.len() - 13, 40] {
        assert!(matches!(
            decode_picture(&png[..cut]),
            Err(PictureReadError::Undecodable {
                format: PictureFormat::Png,
                ..
            })
        ));
    }
    assert!(matches!(
        decode_picture(&JPEG_XL[..20]),
        Err(PictureReadError::UndecodableJpegXl(_))
    ));
}

fn millimetres(value: f64) -> Expression {
    Expression::Measure(value, Unit::Millimetre)
}

fn picture(seed: u8) -> Arc<Picture> {
    let mut bytes = encoded(&DynamicImage::ImageRgba8(quad()), ImageFormat::Png);
    bytes.extend((0..4096_u32).map(|index| (index as u8).wrapping_mul(seed).wrapping_add(seed)));
    read_picture(bytes).unwrap().picture
}

fn image(picture: Arc<Picture>) -> ReferenceImage {
    ReferenceImage {
        picture,
        plane: ImagePlane::Reference(PlaneReference::Principal(PrincipalPlane::Xz)),
        at: [millimetres(5.0), millimetres(2.5)],
        turn: Expression::Measure(30.0, Unit::Degree),
        flipped: true,
        size: ImageSize::Calibrated(Calibration {
            from: ImageSpot::new(0.125, 0.25),
            to: ImageSpot::new(0.875, 0.5),
            distance: millimetres(80.0),
        }),
        opacity: DEFAULT_IMAGE_OPACITY_PERCENT,
        in_front: true,
        locked: true,
    }
}

fn with_images(pictures: &[Arc<Picture>]) -> (Document, Vec<FeatureId>) {
    let mut document = Document::default();
    let mut transaction = document.transaction("Insert");
    let features = pictures
        .iter()
        .enumerate()
        .map(|(index, picture)| {
            transaction.add_feature(
                format!("Photo {index}"),
                FeatureKind::ReferenceImage(Box::new(image(Arc::clone(picture)))),
            )
        })
        .collect();
    document.apply(transaction.finish()).unwrap();
    (document, features)
}

fn carried_pictures(lines: &[String]) -> usize {
    lines
        .iter()
        .map(|line| line.matches("\"data\"").count())
        .sum()
}

#[test]
fn a_reference_image_loads_back_with_each_picture_kept_once_in_the_file() {
    let shared = picture(3);
    let other = picture(7);
    let (document, _) = with_images(&[Arc::clone(&shared), other, shared]);

    let lines = lines_of(&document);
    let loaded = decode_lines(&lines);

    assert_eq!(carried_pictures(&lines), 2);
    assert!(loaded.issues.is_empty(), "{:?}", loaded.issues);
    assert_eq!(loaded.document, document);
}

#[test]
fn a_picture_kept_with_a_damaged_image_loads_as_a_grey_square_and_says_so() {
    let shared = picture(3);
    let (document, _) = with_images(&[Arc::clone(&shared), shared]);
    let lines: Vec<String> = lines_of(&document)
        .into_iter()
        .map(|line| match line.find("\"data\":\"") {
            Some(at) => format!("{}\"data\":\"!!{}", &line[..at], &line[at + 8..]),
            None => line,
        })
        .collect();

    let loaded = decode_lines(&lines);

    assert_eq!(loaded.document.features().count(), 2);
    for feature in loaded.document.features() {
        let FeatureKind::ReferenceImage(image) = &feature.kind else {
            panic!("not an image");
        };
        assert!(image.picture.is_placeholder());
    }
    assert!(loaded.issues.iter().any(|issue| issue
        == "The picture of “Photo 0” could not be read, so it shows as a grey square; insert \
            the picture again."));
    assert!(loaded.issues.iter().any(|issue| issue
        == "The picture of “Photo 1” was kept with another reference image that could not be \
            read, so it shows as a grey square; insert the picture again."));
}

fn set_opacity(document: &Document, feature: FeatureId, opacity: u8) -> Transaction {
    let FeatureKind::ReferenceImage(image) = &document.feature(feature).unwrap().kind else {
        panic!("not an image");
    };
    Transaction::single(
        "Edit",
        Edit::SetFeatureKind {
            id: feature,
            kind: FeatureKind::ReferenceImage(Box::new(ReferenceImage {
                opacity,
                ..image.as_ref().clone()
            })),
        },
    )
}

#[test]
fn the_recovery_journal_writes_a_picture_once_and_replays_every_change_to_it() {
    let dir = TempDir::new().unwrap();
    let base = Document::default();
    let storage = Storage::spawn(config(&dir), untitled(&base), || {}).unwrap();
    let mut editor = Editor::new(base.clone());
    let photo = picture(5);
    let mut insert = editor.document().transaction("Insert");
    let feature = insert.add_feature(
        "Photo",
        FeatureKind::ReferenceImage(Box::new(image(Arc::clone(&photo)))),
    );
    let insert = insert.finish();
    editor.apply(insert.clone()).unwrap();
    storage.record(JournalEntry::Apply(insert)).unwrap();
    for opacity in [20, 40, 80] {
        let change = set_opacity(editor.document(), feature, opacity);
        editor.apply(change.clone()).unwrap();
        storage.record(JournalEntry::Apply(change)).unwrap();
    }
    let undo = editor.next_undo().cloned().unwrap();
    editor.undo().unwrap();
    storage.record(JournalEntry::Undo(undo)).unwrap();
    assert!(storage.flusher().flush(super::WAIT));
    crash(storage);

    let recovery = dir.path().join("recovery");
    let journals: Vec<_> = std::fs::read_dir(&recovery)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    let written = journals
        .iter()
        .map(|path| std::fs::metadata(path).unwrap().len())
        .sum::<u64>();
    let recovered = scan(Some(&recovery), &[]);

    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].editor.document(), editor.document());
    assert!(recovered[0].issues.is_empty());
    assert!(
        written < photo.bytes().len() as u64 * 3 / 2,
        "the journal of {written} bytes holds the {}-byte picture more than once",
        photo.bytes().len()
    );
}

#[test]
fn a_rewritten_journal_keeps_the_pictures_of_its_base_and_entries_once_each() {
    let first = picture(9);
    let (base, features) = with_images(&[Arc::clone(&first)]);
    let mut editor = Editor::new(base.clone());
    let mut entries = Vec::new();
    for opacity in [20, 100] {
        let change = set_opacity(editor.document(), features[0], opacity);
        editor.apply(change.clone()).unwrap();
        entries.push(journal::Logged::Entry(JournalEntry::Apply(change)));
    }
    let head = journal::JournalHead {
        file: None,
        on_disk: None,
        loaded_with_problems: false,
        folded: 0,
    };

    let bytes = journal::encode_journal(&head, &base, &entries).unwrap();
    let contents = journal::decode_journal(&bytes).unwrap();
    let replayed = journal::replay(contents.base, contents.entries);

    assert_eq!(replayed.editor.document(), editor.document());
    assert!(bytes.len() < first.bytes().len() * 3 / 2);
}
