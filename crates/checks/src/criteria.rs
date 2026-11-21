use std::sync::Arc;

use autopilot_core::{Criterion, Verify};

use crate::{Check, Context, Error, Report, Toolbox};

/// The profile command that runs tests. A `test` criterion appends its name to it.
const TEST_COMMAND: &str = "test";

/// Acceptance criteria from the approved plan, each shown to hold by executed
/// evidence. The plan was written by an agent, so it never supplies a command:
/// a `command` criterion names a profile entry, and a `test` criterion supplies
/// only a test name for the profile's own test command.
pub struct CriteriaCheck {
    toolbox: Arc<Toolbox>,
}

impl CriteriaCheck {
    #[must_use]
    pub fn new(toolbox: Arc<Toolbox>) -> Self {
        Self { toolbox }
    }
}

/// A name that cannot be mistaken for a flag or a second argument.
fn usable_test_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 200
        // A leading `@` is a response file for some tools, such as dotnet.
        && !name.starts_with(['-', '@'])
        && name.chars().all(|c| !c.is_whitespace() && !c.is_control())
}

enum Verdict {
    Holds,
    Fails(String),
    /// Cannot be verified, so it cannot pass and no code change fixes it.
    Unknown(String),
}

impl CriteriaCheck {
    fn judge(&self, criterion: &Criterion, context: &Context<'_>) -> Result<Verdict, Error> {
        let id = &criterion.id;
        match criterion.verify {
            Verify::Unknown => Ok(Verdict::Unknown(format!(
                "criterion {id} has no way to be verified"
            ))),
            Verify::Command => {
                let name = criterion.verification_id.as_str();
                match self.toolbox.execute(name, &[], context)? {
                    None => Ok(Verdict::Unknown(format!(
                        "criterion {id} names {name:?}, which is not a command in the profile"
                    ))),
                    Some(done) if done.success => Ok(Verdict::Holds),
                    Some(done) => Ok(Verdict::Fails(format!("criterion {id}: {}", done.note))),
                }
            }
            Verify::Test => {
                let name = criterion.verification_id.as_str();
                if !usable_test_name(name) {
                    return Ok(Verdict::Unknown(format!(
                        "criterion {id} names {name:?}, which is not a usable test name"
                    )));
                }
                let Some(done) = self
                    .toolbox
                    .execute(TEST_COMMAND, &[name.to_owned()], context)?
                else {
                    return Ok(Verdict::Unknown(format!(
                        "criterion {id} needs a {TEST_COMMAND:?} command in the profile"
                    )));
                };
                // A filter that matches nothing still exits 0, so a pass also
                // needs the test's name in the output.
                if done.success && done.text.contains(name) {
                    Ok(Verdict::Holds)
                } else if done.success {
                    Ok(Verdict::Fails(format!(
                        "criterion {id}: no test named {name} ran"
                    )))
                } else {
                    Ok(Verdict::Fails(format!("criterion {id}: {}", done.note)))
                }
            }
        }
    }
}

impl Check for CriteriaCheck {
    fn id(&self) -> &'static str {
        "criteria"
    }

    fn applies(&self, context: &Context<'_>) -> bool {
        !context.plan.criteria.is_empty()
    }

    fn can_refine(&self) -> bool {
        true
    }

    fn configuration(&self) -> String {
        // A criterion can reach any profile command, so all of them count.
        self.toolbox.all_configuration()
    }

    fn run(&self, context: &Context<'_>) -> Result<Report, Error> {
        let mut failures = Vec::new();
        let mut unknowns = Vec::new();
        for criterion in &context.plan.criteria {
            match self.judge(criterion, context)? {
                Verdict::Holds => {}
                Verdict::Fails(note) => failures.push(note),
                Verdict::Unknown(note) => unknowns.push(note),
            }
        }
        let total = context.plan.criteria.len();
        let holding = total - failures.len() - unknowns.len();
        let summary = format!("{holding} of {total} criteria verified");
        if failures.is_empty() && unknowns.is_empty() {
            return Ok(Report::pass(summary));
        }
        let mut note = summary;
        for line in failures.iter().chain(&unknowns) {
            note.push('\n');
            note.push_str(line);
        }
        Ok(Report::fail(note).with_unknowns(unknowns))
    }
}

#[cfg(test)]
mod tests {
    use super::usable_test_name;

    // Invariant: a test name from the plan can only select tests. A name that
    // looks like a flag would let the plan change what the test command does.
    #[test]
    fn a_test_name_cannot_carry_a_flag_or_a_second_argument() {
        assert!(usable_test_name("tests::parses_input"));
        assert!(usable_test_name("math/add.spec"));
        for bad in [
            "",
            "--ignored",
            "-x",
            "@file",
            "two words",
            "a\tb",
            "a\nb",
            "a\u{7}b",
        ] {
            assert!(!usable_test_name(bad), "{bad:?}");
        }
        assert!(!usable_test_name(&"a".repeat(201)));
    }
}
