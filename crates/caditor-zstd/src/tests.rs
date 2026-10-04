use super::*;

fn sample(seed: u64) -> Vec<u8> {
    sample_of(seed, 20_000)
}

fn sample_of(seed: u64, length: usize) -> Vec<u8> {
    let mut state = 0x9e37_79b9_7f4a_7c15_u64 ^ seed;
    (0..length)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % 16) as u8 + b'a'
        })
        .collect()
}

#[test]
fn a_frame_decompresses_to_what_was_compressed() {
    let data = sample(0);
    for level in [Level::FAST, Level::BALANCED, Level::SMALL] {
        let frame = compress(&data, level).unwrap();
        assert!(frame.len() * 3 < data.len() * 2);
        assert_eq!(content_size(&frame).unwrap(), data.len() as u64);
        assert_eq!(decompress(&frame, data.len()).unwrap(), data);
    }
    let empty = compress(&[], Level::FAST).unwrap();
    assert_eq!(decompress(&empty, 0).unwrap(), Vec::<u8>::new());
}

#[test]
fn a_similar_prefix_makes_a_small_delta_that_needs_the_same_prefix_back() {
    let older = sample(0);
    let mut newer = older.clone();
    newer[5_000..5_040].copy_from_slice(&[7; 40]);
    newer.extend_from_slice(b"one more record");

    let alone = compress(&newer, Level::BALANCED).unwrap();
    let delta = compress_after(&newer, &older, Level::BALANCED).unwrap();
    assert!(
        delta.len() * 10 < alone.len(),
        "{} vs {}",
        delta.len(),
        alone.len()
    );
    assert_eq!(
        decompress_after(&delta, &older, newer.len()).unwrap(),
        newer
    );
    assert_ne!(
        decompress_after(&delta, &sample(1), newer.len()).ok(),
        Some(newer)
    );
}

#[test]
fn damaged_or_oversized_frames_are_errors() {
    let data = sample(3);
    let frame = compress(&data, Level::BALANCED).unwrap();
    assert!(matches!(
        decompress(&frame, data.len() - 1),
        Err(ZstdError::TooLarge { .. })
    ));
    assert_eq!(decompress(b"not zstd", 100), Err(ZstdError::NotAFrame));
    assert_eq!(decompress(&[], 100), Err(ZstdError::NotAFrame));
    assert!(decompress(&frame[..frame.len() / 2], data.len()).is_err());
    for position in (8..frame.len()).step_by(frame.len() / 50 + 1) {
        let mut damaged = frame.clone();
        damaged[position] ^= 0x5a;
        if let Ok(content) = decompress(&damaged, data.len()) {
            assert_eq!(content.len(), data.len(), "damage at {position}");
        }
    }
    assert_eq!(Level::new(i32::MAX), None);
    assert_eq!(Level::new(9), Some(Level::BALANCED));
}

#[test]
fn a_frame_without_its_size_is_refused() {
    let data = sample(4);
    let context = Compressor::new().unwrap();
    context
        .set(ZSTD_cParameter::ZSTD_c_contentSizeFlag, 0)
        .unwrap();
    let frame = context.compress(&data, &[]).unwrap();
    assert_eq!(content_size(&frame), Err(ZstdError::UnknownSize));
    assert_eq!(decompress(&frame, usize::MAX), Err(ZstdError::UnknownSize));
}

#[test]
fn a_frame_recording_the_wrong_size_is_refused() {
    let data = sample_of(5, 100);
    let frame = compress(&data, Level::FAST).unwrap();
    let recorded = 5;
    assert_eq!(frame[recorded], 100);
    for claimed in [99_u8, 101] {
        let mut lying = frame.clone();
        lying[recorded] = claimed;
        assert_eq!(content_size(&lying).unwrap(), u64::from(claimed));
        assert!(decompress(&lying, 200).is_err());
    }
}

#[test]
fn a_delta_needs_its_own_prefix_and_an_empty_one_round_trips() {
    let older = sample(6);
    let newer = sample(7);
    let delta = compress_after(&newer, &older, Level::BALANCED).unwrap();
    assert!(decompress(&delta, newer.len()).map_or(true, |content| content != newer));
    assert!(
        decompress_after(&delta, &sample(8), newer.len()).map_or(true, |content| content != newer)
    );
    assert_eq!(
        decompress_after(&delta, &older, newer.len()).unwrap(),
        newer
    );

    let empty = compress_after(&[], &older, Level::BALANCED).unwrap();
    assert_eq!(
        decompress_after(&empty, &older, 0).unwrap(),
        Vec::<u8>::new()
    );
}

#[test]
fn a_delta_reaches_back_across_a_prefix_of_several_mebibytes() {
    let older = sample_of(9, 12 << 20);
    let mut newer = older.clone();
    newer[1_000..1_064].copy_from_slice(&[7; 64]);
    newer.extend_from_slice(b"one more record");

    let delta = compress_after(&newer, &older, Level::BALANCED).unwrap();

    assert!(delta.len() < 64 << 10, "{} bytes", delta.len());
    assert_eq!(
        decompress_after(&delta, &older, newer.len()).unwrap(),
        newer
    );
}

#[test]
fn the_library_names_each_failure_with_its_own_error() {
    let data = sample(3);
    let frame = compress(&data, Level::BALANCED).unwrap();
    let mut ends_wrong = frame.clone();
    let last = ends_wrong.len() - 1;
    ends_wrong[last] ^= 0xff;

    assert_eq!(
        decompress(&frame[..frame.len() / 2], data.len()),
        Err(ZstdError::InputSizeWrong)
    );
    assert_eq!(
        decompress(&ends_wrong, data.len()),
        Err(ZstdError::Corrupted)
    );
    for (code, error) in [
        (code::MEMORY_ALLOCATION, ZstdError::OutOfMemory),
        (code::WORKSPACE_TOO_SMALL, ZstdError::OutOfMemory),
        (code::PREFIX_UNKNOWN, ZstdError::NotAFrame),
        (code::LITERALS_HEADER_WRONG, ZstdError::Corrupted),
        (code::CHECKSUM_WRONG, ZstdError::ChecksumMismatch),
        (code::WINDOW_TOO_LARGE, ZstdError::WindowTooLarge),
        (code::VERSION_UNSUPPORTED, ZstdError::UnsupportedVersion),
        (
            code::FRAME_PARAMETER_UNSUPPORTED,
            ZstdError::UnsupportedFrame,
        ),
        (code::DICTIONARY_WRONG, ZstdError::WrongPrefix),
        (code::DICTIONARY_CORRUPTED, ZstdError::WrongPrefix),
        (code::DESTINATION_TOO_SMALL, ZstdError::OutputTooSmall),
        (code::PARAMETER_UNSUPPORTED, ZstdError::SettingRefused),
        (
            code::PARAMETER_COMBINATION_UNSUPPORTED,
            ZstdError::SettingRefused,
        ),
        (code::PARAMETER_OUT_OF_BOUND, ZstdError::SettingRefused),
        (1, ZstdError::Unclassified { code: 1 }),
    ] {
        assert_eq!(classified(code), error);
    }
}
