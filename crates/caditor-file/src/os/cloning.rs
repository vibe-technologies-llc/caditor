const LARGEST_REQUEST: u64 = 1 << 30;

pub(crate) fn cloneable(from: u64, at: u64, length: u64, cluster: u64) -> u64 {
    if cluster == 0 || !from.is_multiple_of(cluster) || !at.is_multiple_of(cluster) {
        return 0;
    }
    length - length % cluster
}

pub(crate) fn request_length(remaining: u64, cluster: u64) -> u64 {
    let largest = LARGEST_REQUEST - LARGEST_REQUEST % cluster.max(1);
    remaining.min(largest.max(cluster))
}

pub(crate) fn sized_for(end: u64, cluster: u64) -> Option<u64> {
    end.checked_next_multiple_of(cluster)
}

#[cfg(test)]
mod tests {
    use super::{cloneable, request_length, sized_for};

    #[test]
    fn only_whole_clusters_at_aligned_offsets_are_cloned() {
        assert_eq!(cloneable(4096, 8192, 3 * 4096, 4096), 3 * 4096);
        assert_eq!(cloneable(4096, 8192, 3 * 4096 + 100, 4096), 3 * 4096);
        assert_eq!(cloneable(4096, 8192, 4095, 4096), 0);
        assert_eq!(cloneable(0, 0, 0, 4096), 0);
    }

    #[test]
    fn misaligned_offsets_clone_nothing() {
        assert_eq!(cloneable(4096, 8192, 8 * 4096, 65536), 0);
        assert_eq!(cloneable(65536, 8192, 8 * 4096, 65536), 0);
        assert_eq!(cloneable(65536, 131072, 8 * 4096 + 5, 65536), 0);
        assert_eq!(cloneable(65536, 131072, 3 * 65536 + 5, 65536), 3 * 65536);
        assert_eq!(cloneable(0, 0, 10, 0), 0);
    }

    #[test]
    fn requests_are_whole_clusters_and_no_longer_than_the_largest() {
        assert_eq!(request_length(3 * 4096, 4096), 3 * 4096);
        assert_eq!(request_length(u64::MAX, 4096), 1 << 30);
        assert_eq!(request_length(u64::MAX, 65536), 1 << 30);
        assert_eq!(
            request_length(u64::MAX, 3 * 4096),
            (1 << 30) / 12288 * 12288
        );
        assert_eq!(request_length(u64::MAX, 2 << 30), 2 << 30);
    }

    #[test]
    fn a_target_is_sized_to_whole_clusters_past_the_cloned_end() {
        assert_eq!(sized_for(4096, 4096), Some(4096));
        assert_eq!(sized_for(4097, 4096), Some(8192));
        assert_eq!(sized_for(u64::MAX, 4096), None);
    }
}
