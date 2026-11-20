use std::path::Path;

use autopilot_core::Step;
use autopilot_profile::Profile;

#[test]
fn partial_step_overrides_keep_step_specific_defaults() {
    let profile = Profile::parse("project.toml", "[steps.plan]\nmodel = \"planner\"\n").unwrap();

    assert!(profile.step(Step::Plan).unwrap().ask_after);
    assert!(!profile.step(Step::Understand).unwrap().ask_after);
}

#[test]
fn subscription_budget_points_to_the_value_and_key() {
    let error = Profile::parse(
        "project.toml",
        "[steps.plan.limits]\nbudget_microusd = 50000\n",
    )
    .unwrap_err();

    assert_eq!(error.path, Path::new("project.toml"));
    assert_eq!((error.line, error.column), (2, 19));
    assert_eq!(error.key, "steps.plan.limits.budget_microusd");
    assert!(error.message.contains("subscription"));
}

#[test]
fn nested_type_errors_keep_the_profile_key() {
    let error = Profile::parse(
        "project.toml",
        "[steps.build_check.limits]\nturns = \"a lot\"\n",
    )
    .unwrap_err();

    assert_eq!(error.line, 2);
    assert_eq!(error.key, "steps.build_check.limits.turns");
}
