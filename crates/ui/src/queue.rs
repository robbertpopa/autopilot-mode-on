//! Builds wait their turn instead of all starting at once.
//!
//! A Build asks the queue for a slot. At most `ticket_slots` run side by side,
//! and a ticket that predicted a file another running or earlier ticket also
//! predicted waits for that one, so two builds never edit the same file in
//! parallel. The queue lives in memory: a restart empties it, and a Build that
//! was waiting is started again by its owner.

use std::{
    path::PathBuf,
    sync::{Condvar, LazyLock, Mutex, MutexGuard},
};

use autopilot_core::TicketId;
use autopilot_pipeline::scheduler::Scheduler;

pub(crate) struct BuildQueue {
    state: Mutex<Scheduler>,
    changed: Condvar,
}

/// The queue every Build in this window goes through.
pub(crate) static BUILDS: LazyLock<BuildQueue> = LazyLock::new(BuildQueue::new);

struct Slot<'a> {
    queue: &'a BuildQueue,
    ticket: &'a TicketId,
}

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        // Released on every exit, a panic included, or the tickets behind this
        // one would wait for ever.
        let mut state = self.queue.lock();
        state.finish(self.ticket);
        drop(state);
        self.queue.changed.notify_all();
    }
}

impl BuildQueue {
    pub(crate) fn new() -> Self {
        Self {
            state: Mutex::new(Scheduler::new(1)),
            changed: Condvar::new(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Scheduler> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Wait for a free slot and for the tickets that share a file with this one,
    /// then run `work` holding the slot. `paths` must be usable repository
    /// paths (`engine::predicted_paths` returns only those).
    ///
    /// # Errors
    /// Returns the reason when the paths cannot be compared.
    pub(crate) fn run<T>(
        &self,
        ticket: &TicketId,
        paths: Vec<PathBuf>,
        slots: usize,
        work: impl FnOnce() -> T,
    ) -> Result<T, String> {
        {
            let mut state = self.lock();
            state.set_slots(slots);
            if state.running().contains(&ticket)
                || state.waiting().iter().any(|(id, _)| id == ticket)
            {
                return Err("This Build is already running or waiting for a free slot".to_owned());
            }
            state
                .submit(ticket.clone(), paths)
                .map_err(|error| error.to_string())?;
            loop {
                let started = state.start_ready();
                if !started.is_empty() {
                    // Another waiting ticket may be one of them.
                    self.changed.notify_all();
                }
                if state.running().contains(&ticket) {
                    break;
                }
                state = self
                    .changed
                    .wait(state)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
            }
        }
        let _slot = Slot {
            queue: self,
            ticket,
        };
        Ok(work())
    }

    /// Why this ticket's Build has not started, in words for the ticket page.
    pub(crate) fn waiting(&self, ticket: &TicketId) -> Option<String> {
        self.lock()
            .waiting()
            .into_iter()
            .find(|(id, _)| id == ticket)
            .map(|(_, wait)| wait.describe())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use std::{
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
            mpsc,
        },
        thread,
        time::{Duration, Instant},
    };

    use super::*;

    fn id(name: &str) -> TicketId {
        TicketId::from(name)
    }

    fn until(what: &str, mut done: impl FnMut() -> bool) {
        let limit = Instant::now() + Duration::from_secs(10);
        while !done() {
            assert!(Instant::now() < limit, "timed out waiting for: {what}");
            thread::sleep(Duration::from_millis(5));
        }
    }

    /// A Build that holds its slot until it is told to finish.
    fn hold(
        queue: &Arc<BuildQueue>,
        name: &str,
        paths: &[&str],
        slots: usize,
        started: &Arc<AtomicUsize>,
    ) -> (mpsc::Sender<()>, thread::JoinHandle<()>) {
        let (release, wait) = mpsc::channel::<()>();
        let queue = Arc::clone(queue);
        let started = Arc::clone(started);
        let ticket = id(name);
        let paths: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
        let handle = thread::spawn(move || {
            queue
                .run(&ticket, paths, slots, || {
                    started.fetch_add(1, Ordering::SeqCst);
                    let _ = wait.recv();
                })
                .unwrap();
        });
        (release, handle)
    }

    // Invariant: with two slots, three tickets run two at a time and the third
    // starts only when one finishes. Breaks if the queue starts every Build at
    // once, or never hands a freed slot to the waiting ticket.
    #[test]
    fn three_tickets_two_slots_the_third_waits_for_a_free_one() {
        let queue = Arc::new(BuildQueue::new());
        let started = Arc::new(AtomicUsize::new(0));
        let (release_a, a) = hold(&queue, "a", &["src/a.rs"], 2, &started);
        let (release_b, b) = hold(&queue, "b", &["src/b.rs"], 2, &started);
        until("two builds running", || started.load(Ordering::SeqCst) == 2);
        let (release_c, c) = hold(&queue, "c", &["src/c.rs"], 2, &started);
        until("the third is queued", || queue.waiting(&id("c")).is_some());
        assert_eq!(
            queue.waiting(&id("c")).unwrap(),
            "waits for a free slot (2 of 2 running)"
        );
        assert_eq!(started.load(Ordering::SeqCst), 2);

        release_a.send(()).unwrap();
        until("the third starts", || started.load(Ordering::SeqCst) == 3);
        assert!(queue.waiting(&id("c")).is_none());
        release_b.send(()).unwrap();
        release_c.send(()).unwrap();
        for handle in [a, b, c] {
            handle.join().unwrap();
        }
    }

    // Invariant: a ticket that predicts a file a running ticket also predicts is
    // queued, not run, even with a free slot, and runs once that one finishes.
    // Breaks if the paths are not passed to the scheduler.
    #[test]
    fn a_collision_is_queued_not_run() {
        let queue = Arc::new(BuildQueue::new());
        let started = Arc::new(AtomicUsize::new(0));
        let (release_a, a) = hold(&queue, "a", &["src/export.rs"], 3, &started);
        until("the first build runs", || {
            started.load(Ordering::SeqCst) == 1
        });
        let (release_b, b) = hold(&queue, "b", &["src/export.rs"], 3, &started);
        until("the second is queued", || queue.waiting(&id("b")).is_some());
        assert_eq!(
            queue.waiting(&id("b")).unwrap(),
            "waits for #a: both change src/export.rs"
        );
        assert_eq!(
            started.load(Ordering::SeqCst),
            1,
            "a free slot is not enough"
        );

        release_a.send(()).unwrap();
        until("the second starts", || started.load(Ordering::SeqCst) == 2);
        release_b.send(()).unwrap();
        a.join().unwrap();
        b.join().unwrap();
    }

    // Invariant: a Build that fails or panics gives its slot back, so the queue
    // does not jam. Breaks if the slot is released only on success.
    #[test]
    fn a_panicking_build_still_frees_its_slot() {
        let queue = Arc::new(BuildQueue::new());
        let crashed = {
            let queue = Arc::clone(&queue);
            thread::spawn(move || {
                let _ = queue.run(&id("a"), vec![], 1, || panic!("build crashed"));
            })
            .join()
        };
        assert!(crashed.is_err());
        assert_eq!(queue.run(&id("b"), vec![], 1, || 5).unwrap(), 5);
    }

    // Invariant: pressing Build twice does not queue the same ticket twice.
    #[test]
    fn the_same_ticket_is_not_queued_twice() {
        let queue = Arc::new(BuildQueue::new());
        let started = Arc::new(AtomicUsize::new(0));
        let (release, handle) = hold(&queue, "a", &[], 2, &started);
        until("it runs", || started.load(Ordering::SeqCst) == 1);
        assert!(queue.run(&id("a"), vec![], 2, || ()).is_err());
        release.send(()).unwrap();
        handle.join().unwrap();
    }
}
