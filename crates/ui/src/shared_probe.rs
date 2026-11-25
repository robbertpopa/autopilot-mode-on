//! One sandbox probe for the Builds that start together.
//!
//! The probe takes about five seconds and every Build asks for it, so three
//! tickets started at once would run it three times. The first caller runs it;
//! the others wait on the lock and reuse the answer. An answer is only reused
//! for a short while: a Build that starts minutes later probes again, because
//! the boundary may have changed since.

use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

/// How long a passed probe may be reused. Long enough for Builds started in the
/// same moment, too short to hide a sandbox that was changed afterwards.
pub(crate) const REUSE_FOR: Duration = Duration::from_secs(60);

pub(crate) struct SharedProbe<T> {
    last: Mutex<Option<(Instant, T)>>,
}

impl<T: Clone> SharedProbe<T> {
    pub(crate) const fn new() -> Self {
        Self {
            last: Mutex::new(None),
        }
    }

    /// The last passed probe if it is younger than `REUSE_FOR`, else the result
    /// of `probe`. The lock is held while probing, so callers never probe side
    /// by side. A failure is never kept: the next Build probes again.
    pub(crate) fn get(&self, probe: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
        self.get_at(Instant::now(), probe)
    }

    fn get_at(&self, now: Instant, probe: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
        // A panic in another probe leaves nothing half-written worth keeping.
        let mut last = self
            .last
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((at, value)) = last.as_ref()
            && now.saturating_duration_since(*at) < REUSE_FOR
        {
            return Ok(value.clone());
        }
        *last = None;
        let value = probe()?;
        *last = Some((Instant::now(), value.clone()));
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // Invariant: Builds that start together run the probe once, not once each.
    // Breaks if: the lock is dropped before probing or the answer is not kept;
    // no other test counts probe runs.
    #[test]
    fn builds_started_together_probe_once() {
        let shared = SharedProbe::<u32>::new();
        let runs = AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..3 {
                scope.spawn(|| {
                    let got = shared.get(|| {
                        runs.fetch_add(1, Ordering::SeqCst);
                        std::thread::sleep(Duration::from_millis(50));
                        Ok(7)
                    });
                    assert_eq!(got, Ok(7));
                });
            }
        });
        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }

    // Invariant: a later Build checks the boundary again.
    // Breaks if: the age check is removed (a stale pass would be trusted).
    #[test]
    fn an_old_pass_is_probed_again() {
        let shared = SharedProbe::<u32>::new();
        let start = Instant::now();
        assert_eq!(shared.get_at(start, || Ok(1)), Ok(1));
        let later = start + REUSE_FOR + Duration::from_secs(1);
        assert_eq!(shared.get_at(later, || Ok(2)), Ok(2));
    }

    // Invariant: a refusal is never remembered, so fixing the sandbox and
    // pressing Build again works at once.
    // Breaks if: an Err is stored as the answer.
    #[test]
    fn a_refusal_is_not_kept() {
        let shared = SharedProbe::<u32>::new();
        assert_eq!(
            shared.get(|| Err("row 3 failed".into())),
            Err("row 3 failed".into())
        );
        assert_eq!(shared.get(|| Ok(5)), Ok(5));
    }
}
