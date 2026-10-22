use caditor_document::CancelToken;

use super::*;

#[test]
fn every_item_is_worked_in_order_even_once_cancelled() {
    let items: Vec<usize> = (0..1000).collect();
    let cancelled = CancelToken::new(|| true);

    let stopped = interruptible(cancelled.interrupt(), || {
        in_parallel(&items, |item| item * 2)
    });
    let each = interruptible(cancelled.interrupt(), || {
        each_in_parallel(&items, |item| item * 2)
    });

    assert!(stopped.len() < items.len());
    assert_eq!(each, items.iter().map(|item| item * 2).collect::<Vec<_>>());
}
