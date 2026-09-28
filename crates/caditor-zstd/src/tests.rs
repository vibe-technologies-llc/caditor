use super::*;

fn sample(seed: u64) -> Vec<u8> {
    let mut state = 0x9e37_79b9_7f4a_7c15_u64 ^ seed;
    (0..20_000)
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
    assert!(decompress(&frame[..frame.len() / 2], data.len()).is_err());
    for position in (8..frame.len()).step_by(frame.len() / 50 + 1) {
        let mut damaged = frame.clone();
        damaged[position] ^= 0x5a;
        let _ = decompress(&damaged, data.len());
    }
    assert_eq!(Level::new(i32::MAX), None);
    assert_eq!(Level::new(9), Some(Level::BALANCED));
}
