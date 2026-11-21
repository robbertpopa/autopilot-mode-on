use crate::{Check, Context, Error, Report};

/// An agent review in a fresh context. The trait lives here so `checks` stays
/// below the agent adapters; the engine supplies the implementation.
pub trait Reviewer {
    /// # Errors
    /// Returns an error when the review could not run at all.
    fn review(&self, context: &Context<'_>) -> Result<Report, Error>;
    /// The model, effort and prompt that shape the answer.
    fn configuration(&self) -> String;
}

pub struct ReviewCheck {
    reviewer: Box<dyn Reviewer>,
}

impl ReviewCheck {
    #[must_use]
    pub fn new(reviewer: Box<dyn Reviewer>) -> Self {
        Self { reviewer }
    }
}

impl Check for ReviewCheck {
    fn id(&self) -> &'static str {
        "review"
    }

    fn applies(&self, _: &Context<'_>) -> bool {
        true
    }

    fn can_refine(&self) -> bool {
        true
    }

    fn configuration(&self) -> String {
        self.reviewer.configuration()
    }

    fn run(&self, context: &Context<'_>) -> Result<Report, Error> {
        self.reviewer.review(context)
    }
}
