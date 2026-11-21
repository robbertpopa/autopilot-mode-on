//! The Analyze pass: a read-only check of a plan before the owner approves it.
//! Code makes the findings, so they are repeatable and cost nothing.

use autopilot_core::{Plan, Verify};

/// Words that name a goal without a way to check it.
const VAGUE: [&str; 9] = [
    "properly",
    "correctly",
    "appropriate",
    "as expected",
    "user-friendly",
    "intuitive",
    "robust",
    "fast",
    "seamless",
];
const MAX_FINDINGS: usize = 50;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Finding {
    pub severity: Severity,
    /// The criterion the finding is about, if any.
    pub criterion: Option<String>,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Analysis {
    pub findings: Vec<Finding>,
    /// Each criterion and whether the test plan names its verification.
    pub coverage: Vec<(String, bool)>,
}

impl Analysis {
    /// An error means the plan cannot be checked as written.
    #[must_use]
    pub fn blocking(&self) -> bool {
        self.findings
            .iter()
            .any(|finding| finding.severity == Severity::Error)
    }

    #[must_use]
    pub fn count(&self, severity: Severity) -> usize {
        self.findings
            .iter()
            .filter(|finding| finding.severity == severity)
            .count()
    }

    /// One line for the approval request and for `status`.
    #[must_use]
    pub fn describe(&self) -> String {
        format!(
            "{} errors, {} warnings, {} of {} criteria covered by the test plan",
            self.count(Severity::Error),
            self.count(Severity::Warning),
            self.coverage.iter().filter(|(_, covered)| *covered).count(),
            self.coverage.len(),
        )
    }
}

fn finding(severity: Severity, criterion: Option<&str>, message: impl Into<String>) -> Finding {
    Finding {
        severity,
        criterion: criterion.map(str::to_owned),
        message: message.into(),
    }
}

#[must_use]
pub fn analyze(plan: &Plan) -> Analysis {
    let mut findings = Vec::new();
    let mut coverage = Vec::new();
    let test_plan = plan.test_plan.join("\n").to_lowercase();
    for criterion in &plan.criteria {
        let id = Some(criterion.id.as_str());
        if criterion.verify == Verify::Unknown {
            findings.push(finding(
                Severity::Error,
                id,
                "no verification: an unknown criterion can never pass a check",
            ));
        } else if criterion.verification_id.trim().is_empty() {
            findings.push(finding(
                Severity::Error,
                id,
                "it says how it is verified but not by which test or command",
            ));
        }
        let text = criterion.text.to_lowercase();
        for word in VAGUE.iter().filter(|word| text.contains(**word)) {
            findings.push(finding(
                Severity::Warning,
                id,
                format!("vague wording {word:?}: say what is observed instead"),
            ));
        }
        let verification = criterion.verification_id.trim().to_lowercase();
        let covered = !verification.is_empty() && test_plan.contains(&verification);
        if criterion.verify != Verify::Unknown && !covered {
            findings.push(finding(
                Severity::Warning,
                id,
                "the test plan does not mention its verification",
            ));
        }
        coverage.push((criterion.id.clone(), covered));
    }
    if plan.files.is_empty() {
        findings.push(finding(Severity::Info, None, "the plan lists no files"));
    }
    // Errors first, so a cut-off list never hides one.
    findings.sort_by_key(|f| f.severity);
    findings.truncate(MAX_FINDINGS);
    Analysis { findings, coverage }
}

#[cfg(test)]
mod tests {
    use super::*;
    use autopilot_core::Criterion;

    fn criterion(id: &str, text: &str, verify: Verify, verification: &str) -> Criterion {
        Criterion {
            id: id.into(),
            text: text.into(),
            verify,
            verification_id: verification.into(),
        }
    }

    fn plan(criteria: Vec<Criterion>, test_plan: &[&str]) -> Plan {
        Plan {
            criteria,
            test_plan: test_plan.iter().map(|s| (*s).to_owned()).collect(),
            files: vec!["src/export.rs".into()],
        }
    }

    // Invariant: a criterion with no way to verify it blocks approval. If it
    // did not, "unknown" criteria would reach Build and could never pass.
    #[test]
    fn unknown_or_unnamed_verification_blocks() {
        let analysis = analyze(&plan(
            vec![
                criterion("c1", "Export lists every ticket", Verify::Unknown, ""),
                criterion("c2", "Export is quoted", Verify::Test, " "),
            ],
            &[],
        ));
        assert!(analysis.blocking());
        assert_eq!(analysis.count(Severity::Error), 2);
    }

    #[test]
    fn vague_words_and_coverage_gaps_are_warnings() {
        let analysis = analyze(&plan(
            vec![
                criterion(
                    "c1",
                    "The export works properly",
                    Verify::Test,
                    "export_test",
                ),
                criterion(
                    "c2",
                    "Headers are written",
                    Verify::Command,
                    "cargo test headers",
                ),
            ],
            &["Run export_test for the happy path"],
        ));
        assert!(!analysis.blocking());
        assert_eq!(
            analysis.coverage,
            vec![("c1".into(), true), ("c2".into(), false)]
        );
        let warnings: Vec<_> = analysis
            .findings
            .iter()
            .filter(|f| f.severity == Severity::Warning)
            .collect();
        assert_eq!(warnings.len(), 2);
        assert!(
            warnings[0].message.contains("properly") || warnings[1].message.contains("properly")
        );
        assert_eq!(
            analysis.describe(),
            "0 errors, 2 warnings, 1 of 2 criteria covered by the test plan"
        );
    }

    // Invariant: at most 50 findings, and errors survive the cut.
    #[test]
    fn findings_are_capped_with_errors_kept() {
        let mut criteria: Vec<Criterion> = (0..29)
            .map(|i| {
                criterion(
                    &format!("w{i}"),
                    "works properly and is fast",
                    Verify::Test,
                    "t",
                )
            })
            .collect();
        criteria.push(criterion("bad", "Something", Verify::Unknown, ""));
        let analysis = analyze(&plan(criteria, &[]));
        assert_eq!(analysis.findings.len(), 50);
        assert_eq!(analysis.findings[0].severity, Severity::Error);
    }

    #[test]
    fn a_clean_plan_has_no_findings() {
        let analysis = analyze(&plan(
            vec![criterion(
                "c1",
                "Row count equals ticket count",
                Verify::Test,
                "row_count",
            )],
            &["row_count compares both"],
        ));
        assert_eq!(analysis.findings, vec![]);
    }
}
