//! Clarify: turn the open questions of an Understand result into `Q → A` pairs.

use autopilot_core::{OpenQuestion, Understanding};
use serde::{Deserialize, Serialize};

/// Skip reason recorded when Understand left nothing to ask.
pub const NO_OPEN_QUESTIONS: &str = "no open questions";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct QaPair {
    pub question: String,
    pub answer: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ClarifyPlan {
    Skip(&'static str),
    Ask(Vec<OpenQuestion>),
}

/// How the owner answered one question.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Choice {
    /// Take the model's recommended option.
    Recommended,
    Option(usize),
    Text(String),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ClarifyError {
    #[error("{given} answers for {expected} questions")]
    WrongCount { expected: usize, given: usize },
    #[error("question {0} has no such option")]
    NoSuchOption(usize),
    #[error("the free-text answer to question {0} is empty")]
    EmptyText(usize),
}

#[must_use]
pub fn plan(understanding: &Understanding) -> ClarifyPlan {
    if understanding.open_questions.is_empty() {
        ClarifyPlan::Skip(NO_OPEN_QUESTIONS)
    } else {
        ClarifyPlan::Ask(understanding.open_questions.clone())
    }
}

/// Accepting every recommendation is one action.
#[must_use]
pub fn accept_recommended(questions: &[OpenQuestion]) -> Vec<QaPair> {
    questions
        .iter()
        .map(|question| pair(question, question.options.get(question.recommended)))
        .collect()
}

/// # Errors
/// Returns an error if the answers do not line up with the questions.
pub fn resolve(
    questions: &[OpenQuestion],
    choices: &[Choice],
) -> Result<Vec<QaPair>, ClarifyError> {
    if questions.len() != choices.len() {
        return Err(ClarifyError::WrongCount {
            expected: questions.len(),
            given: choices.len(),
        });
    }
    let mut pairs = Vec::with_capacity(questions.len());
    for (index, (question, choice)) in questions.iter().zip(choices).enumerate() {
        let answer = match choice {
            Choice::Recommended => question.options.get(question.recommended),
            Choice::Option(chosen) => Some(
                question
                    .options
                    .get(*chosen)
                    .ok_or(ClarifyError::NoSuchOption(index))?,
            ),
            Choice::Text(text) if text.trim().is_empty() => {
                return Err(ClarifyError::EmptyText(index));
            }
            Choice::Text(text) => {
                pairs.push(QaPair {
                    question: question.question.clone(),
                    answer: text.trim().to_owned(),
                });
                continue;
            }
        };
        pairs.push(pair(question, answer));
    }
    Ok(pairs)
}

// `validate` guarantees the recommendation exists; an unchecked result yields
// an empty answer rather than a panic.
fn pair(question: &OpenQuestion, answer: Option<&String>) -> QaPair {
    QaPair {
        question: question.question.clone(),
        answer: answer.cloned().unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use autopilot_core::{Ambiguity, Clarity};

    fn question(recommended: usize) -> OpenQuestion {
        OpenQuestion {
            question: "Delimiter?".into(),
            options: vec!["Comma".into(), "Semicolon".into()],
            recommended,
        }
    }

    fn understanding(questions: Vec<OpenQuestion>) -> Understanding {
        Understanding {
            summary: "s".into(),
            ambiguity: Ambiguity::all(Clarity::Clear),
            open_questions: questions,
            assumptions: vec![],
            needs_design: false,
            ui_files: vec![],
            files_touched: vec![],
        }
    }

    // Invariant: no questions means Clarify is skipped with the exact reason
    // the log and the window show. Asking nothing would stall the ticket.
    #[test]
    fn no_open_questions_skips_with_the_recorded_reason() {
        assert_eq!(
            plan(&understanding(vec![])),
            ClarifyPlan::Skip("no open questions")
        );
        assert!(matches!(
            plan(&understanding(vec![question(0)])),
            ClarifyPlan::Ask(q) if q.len() == 1
        ));
    }

    #[test]
    fn recommended_is_one_action_and_matches_per_question_choice() {
        let questions = vec![question(1), question(0)];
        let all = accept_recommended(&questions);
        assert_eq!(all[0].answer, "Semicolon");
        assert_eq!(all[1].answer, "Comma");
        assert_eq!(
            resolve(&questions, &[Choice::Recommended, Choice::Recommended]).unwrap(),
            all
        );
    }

    #[test]
    fn explicit_choices_and_free_text_are_recorded_as_pairs() {
        let questions = vec![question(0), question(0)];
        let pairs = resolve(
            &questions,
            &[Choice::Option(1), Choice::Text("  Tab ".into())],
        )
        .unwrap();
        assert_eq!(pairs[0].answer, "Semicolon");
        assert_eq!(pairs[1].answer, "Tab");
    }

    // Invariant: a malformed answer never becomes an empty or wrong pair.
    #[test]
    fn bad_answers_are_rejected() {
        let questions = vec![question(0)];
        assert_eq!(
            resolve(&questions, &[]),
            Err(ClarifyError::WrongCount {
                expected: 1,
                given: 0
            })
        );
        assert_eq!(
            resolve(&questions, &[Choice::Option(2)]),
            Err(ClarifyError::NoSuchOption(0))
        );
        assert_eq!(
            resolve(&questions, &[Choice::Text(" ".into())]),
            Err(ClarifyError::EmptyText(0))
        );
    }
}
