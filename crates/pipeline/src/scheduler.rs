//! Which queued tickets may start. Pure state: the caller starts what `next`
//! returns and calls `finish` when a ticket stops holding a slot.
//!
//! Order is first come, first served, except that a ticket which cannot start
//! never blocks an independent one behind it. A ticket does wait for any
//! earlier queued ticket it overlaps, so two tickets that change the same file
//! run in the order they arrived instead of the later one slipping ahead.

use std::path::PathBuf;

use autopilot_core::TicketId;
use autopilot_git::overlapping_paths;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("the predicted paths are not usable: {0}")]
    Paths(#[from] autopilot_git::Error),
}

/// Why a queued ticket has not started.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Wait {
    Paused,
    /// Every slot is taken.
    Slots {
        running: usize,
        limit: usize,
    },
    /// It overlaps a running ticket, or an earlier one still in the queue.
    Overlap {
        with: TicketId,
        path: PathBuf,
        started: bool,
    },
}

impl Wait {
    /// The sentence shown next to the ticket.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::Paused => "the queue is paused".into(),
            Self::Slots { running, limit } => {
                format!("waits for a free slot ({running} of {limit} running)")
            }
            Self::Overlap { with, path, .. } => format!(
                "waits for #{}: both change {}",
                with.as_str(),
                path.display().to_string().replace('\\', "/")
            ),
        }
    }
}

struct Entry {
    ticket: TicketId,
    paths: Vec<PathBuf>,
}

pub struct Scheduler {
    slots: usize,
    paused: bool,
    running: Vec<Entry>,
    queue: Vec<Entry>,
}

impl Scheduler {
    /// A scheduler that runs at most `slots` tickets at once. Zero is read as one.
    #[must_use]
    pub fn new(slots: usize) -> Self {
        Self {
            slots: slots.max(1),
            paused: false,
            running: Vec::new(),
            queue: Vec::new(),
        }
    }

    /// Queue a ticket with the paths Understand and Plan predict it will change.
    /// A ticket already queued or running is left as it is.
    ///
    /// # Errors
    /// Returns an error if a predicted path is empty, absolute or leaves the repository.
    pub fn submit(&mut self, ticket: TicketId, paths: Vec<PathBuf>) -> Result<(), Error> {
        overlapping_paths(&paths, &[])?;
        if self.knows(&ticket) {
            return Ok(());
        }
        self.queue.push(Entry { ticket, paths });
        Ok(())
    }

    fn knows(&self, ticket: &TicketId) -> bool {
        self.running
            .iter()
            .chain(&self.queue)
            .any(|entry| &entry.ticket == ticket)
    }

    /// Stop handing out starts. Running tickets are not touched.
    pub fn pause(&mut self) {
        self.paused = true;
    }

    pub fn resume(&mut self) {
        self.paused = false;
    }

    #[must_use]
    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// A ticket stopped holding a slot: it finished, failed or was cancelled.
    /// A queued ticket is simply removed.
    pub fn finish(&mut self, ticket: &TicketId) {
        self.running.retain(|entry| &entry.ticket != ticket);
        self.queue.retain(|entry| &entry.ticket != ticket);
    }

    #[must_use]
    pub fn running(&self) -> Vec<&TicketId> {
        self.running.iter().map(|entry| &entry.ticket).collect()
    }

    /// Move every ticket that may start now into the running set, in queue order.
    pub fn start_ready(&mut self) -> Vec<TicketId> {
        let mut started = Vec::new();
        if self.paused {
            return started;
        }
        let mut index = 0;
        while index < self.queue.len() && self.running.len() < self.slots {
            if self.blocker(index).is_none() {
                let entry = self.queue.remove(index);
                started.push(entry.ticket.clone());
                self.running.push(entry);
            } else {
                index += 1;
            }
        }
        started
    }

    /// Why each queued ticket is waiting, in queue order. A ticket that could
    /// start is not listed.
    #[must_use]
    pub fn waiting(&self) -> Vec<(TicketId, Wait)> {
        (0..self.queue.len())
            .filter_map(|index| {
                self.reason(index)
                    .map(|wait| (self.queue[index].ticket.clone(), wait))
            })
            .collect()
    }

    fn reason(&self, index: usize) -> Option<Wait> {
        if self.paused {
            return Some(Wait::Paused);
        }
        // An overlap is the more useful answer, so it is reported first.
        self.blocker(index).or_else(|| {
            (self.running.len() >= self.slots).then_some(Wait::Slots {
                running: self.running.len(),
                limit: self.slots,
            })
        })
    }

    /// The first running ticket, then the first earlier queued one, that the
    /// ticket at `index` overlaps.
    fn blocker(&self, index: usize) -> Option<Wait> {
        let paths = &self.queue[index].paths;
        let ahead = self.queue[..index].iter().map(|entry| (entry, false));
        self.running
            .iter()
            .map(|entry| (entry, true))
            .chain(ahead)
            .find_map(|(entry, started)| {
                // Paths were validated on submit, so a failure here cannot happen.
                let overlaps = overlapping_paths(paths, &entry.paths).ok()?;
                let first = overlaps.into_iter().next()?;
                Some(Wait::Overlap {
                    with: entry.ticket.clone(),
                    path: first.proposed,
                    started,
                })
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(name: &str) -> TicketId {
        TicketId::from(name)
    }

    fn paths(list: &[&str]) -> Vec<PathBuf> {
        list.iter().map(PathBuf::from).collect()
    }

    fn started(scheduler: &mut Scheduler) -> Vec<String> {
        scheduler
            .start_ready()
            .iter()
            .map(|ticket| ticket.as_str().to_owned())
            .collect()
    }

    // Invariant: independent tickets run side by side up to the slot limit,
    // and the rest wait for a slot, not for each other.
    #[test]
    fn independent_tickets_run_in_parallel_up_to_the_slot_limit() {
        let mut scheduler = Scheduler::new(2);
        for (name, file) in [("1", "a.rs"), ("2", "b.rs"), ("3", "c.rs")] {
            scheduler.submit(id(name), paths(&[file])).unwrap();
        }
        assert_eq!(started(&mut scheduler), ["1", "2"]);
        let waiting = scheduler.waiting();
        assert_eq!(waiting.len(), 1);
        assert_eq!(
            waiting[0].1.describe(),
            "waits for a free slot (2 of 2 running)"
        );
        scheduler.finish(&id("1"));
        assert_eq!(started(&mut scheduler), ["3"]);
    }

    // Invariant: two tickets that change one file run in arrival order, and
    // the reason names the other ticket and the shared path.
    #[test]
    fn tickets_touching_one_file_queue_and_run_in_order() {
        let mut scheduler = Scheduler::new(4);
        scheduler
            .submit(id("127"), paths(&["src/routes.rs"]))
            .unwrap();
        scheduler
            .submit(id("128"), paths(&["src/routes.rs", "src/other.rs"]))
            .unwrap();
        assert_eq!(started(&mut scheduler), ["127"]);
        assert_eq!(
            scheduler.waiting()[0].1.describe(),
            "waits for #127: both change src/routes.rs"
        );
        scheduler.finish(&id("127"));
        assert_eq!(started(&mut scheduler), ["128"]);
    }

    // Invariant: a blocked ticket does not hold up an independent one behind
    // it, and a later ticket cannot overtake an earlier one it overlaps.
    #[test]
    fn a_blocked_ticket_does_not_block_others_but_is_not_overtaken() {
        let mut scheduler = Scheduler::new(4);
        scheduler.submit(id("1"), paths(&["src"])).unwrap();
        scheduler.submit(id("2"), paths(&["src/a.rs"])).unwrap();
        scheduler.submit(id("3"), paths(&["docs"])).unwrap();
        scheduler.submit(id("4"), paths(&["src/a.rs"])).unwrap();
        assert_eq!(started(&mut scheduler), ["1", "3"]);
        let waiting = scheduler.waiting();
        assert_eq!(waiting.len(), 2);
        assert!(matches!(
            &waiting[1].1,
            Wait::Overlap { with, started: true, .. } if with == &id("1")
        ));
        scheduler.finish(&id("1"));
        assert_eq!(started(&mut scheduler), ["2"]);
        assert!(matches!(
            &scheduler.waiting()[0].1,
            Wait::Overlap { with, started: true, .. } if with == &id("2")
        ));
        scheduler.finish(&id("2"));
        assert_eq!(started(&mut scheduler), ["4"]);
    }

    // Invariant: pause stops new starts without touching running tickets, and
    // resume lets the queue go on.
    #[test]
    fn pause_holds_the_whole_queue_and_resume_releases_it() {
        let mut scheduler = Scheduler::new(2);
        scheduler.submit(id("1"), paths(&["a.rs"])).unwrap();
        assert_eq!(started(&mut scheduler), ["1"]);
        scheduler.pause();
        scheduler.submit(id("2"), paths(&["b.rs"])).unwrap();
        assert_eq!(started(&mut scheduler), Vec::<String>::new());
        assert_eq!(scheduler.waiting()[0].1, Wait::Paused);
        assert_eq!(scheduler.running(), [&id("1")]);
        scheduler.resume();
        assert_eq!(started(&mut scheduler), ["2"]);
    }

    // Invariant: submitting twice never runs a ticket twice, and a prediction
    // that cannot be compared is refused up front.
    #[test]
    fn a_ticket_is_queued_once_and_bad_paths_are_refused() {
        let mut scheduler = Scheduler::new(2);
        scheduler.submit(id("1"), paths(&["a.rs"])).unwrap();
        scheduler.submit(id("1"), paths(&["a.rs"])).unwrap();
        assert_eq!(started(&mut scheduler), ["1"]);
        scheduler.submit(id("1"), paths(&["a.rs"])).unwrap();
        assert_eq!(started(&mut scheduler), Vec::<String>::new());
        assert!(scheduler.submit(id("2"), paths(&["/etc/passwd"])).is_err());
        assert!(scheduler.submit(id("3"), paths(&["../x"])).is_err());
        assert_eq!(scheduler.waiting(), Vec::new());
    }

    // Invariant: a cancelled queued ticket leaves no trace that could block others.
    #[test]
    fn finishing_a_queued_ticket_removes_it() {
        let mut scheduler = Scheduler::new(1);
        scheduler.submit(id("1"), paths(&["a.rs"])).unwrap();
        scheduler.submit(id("2"), paths(&["a.rs"])).unwrap();
        scheduler.submit(id("3"), paths(&["a.rs"])).unwrap();
        assert_eq!(started(&mut scheduler), ["1"]);
        scheduler.finish(&id("2"));
        scheduler.finish(&id("1"));
        assert_eq!(started(&mut scheduler), ["3"]);
    }
}
