use std::sync::atomic::{AtomicUsize, Ordering};

/// Host slots claim work as soon as they become free. Returning input order
/// keeps result folding deterministic without delaying queued executions.
pub(crate) fn map<T: Sync, R: Send>(
    items: &[T],
    forks: usize,
    work: impl Fn(&T) -> R + Sync,
) -> Vec<R> {
    assert!(forks > 0, "Host scheduler requires a positive forks limit");
    let next = AtomicUsize::new(0);
    let mut results = std::thread::scope(|scope| {
        let workers = (0..forks.min(items.len()))
            .map(|_| {
                scope.spawn(|| {
                    let mut completed = Vec::new();
                    loop {
                        let index = next.fetch_add(1, Ordering::Relaxed);
                        let Some(item) = items.get(index) else { break };
                        completed.push((index, work(item)));
                    }
                    completed
                })
            })
            .collect::<Vec<_>>();
        workers
            .into_iter()
            .flat_map(|worker| {
                worker
                    .join()
                    .expect("Host scheduler panicked outside task execution")
            })
            .collect::<Vec<_>>()
    });
    results.sort_by_key(|(index, _)| *index);
    results.into_iter().map(|(_, result)| result).collect()
}
