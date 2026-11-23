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

// Invariant: a profile saved before projects and pipelines existed still
// loads, and a ticket with no choice runs `full`. This is a literal old file,
// not a freshly written one, so a new required field would fail here.
#[test]
fn an_old_profile_has_no_projects_and_only_the_built_in_pipelines() {
    let old = "protected_paths = [\"infra/**\"]\nwhen_unsure = \"ask\"\n\n[steps.plan]\nmodel = \"planner\"\n\n[budgets]\nper_ticket_microusd = 500000\n";
    let profile = Profile::parse("old.toml", old).unwrap();
    assert!(profile.projects.is_empty());
    let names: Vec<String> = profile.pipelines().into_iter().map(|p| p.name).collect();
    assert_eq!(
        names,
        ["full", "bug-fix", "quick-change", "ui-change", "ask-only"]
    );
    assert_eq!(profile.pipeline("full").unwrap().off().len(), 0);
}

// Invariant: a project can add protected paths and lower budgets, and
// nothing it says can empty the global list or raise a cap. An empty project
// list or a larger project budget must leave the global rule as it was.
#[test]
fn a_project_never_loosens_protected_paths_or_budgets() {
    let source = r#"
protected_paths = ["infra/**"]
[budgets]
per_ticket_microusd = 500000

[projects.'D:\work\app']
protected_paths = []
[projects.'D:\work\app'.budgets]
per_ticket_microusd = 9000000
daily_microusd = 100000

[projects."D:/work/tight"]
protected_paths = ["secrets/**"]
[projects."D:/work/tight".budgets]
per_ticket_microusd = 100000
"#;
    let profile = Profile::parse("p.toml", source).unwrap();
    let loose = profile.for_project(Some("D:/work/app/"));
    assert_eq!(loose.protected_paths, ["infra/**"]);
    assert_eq!(loose.budgets.per_ticket_microusd.unwrap().get(), 500_000);
    // A cap beats no cap for a budget the profile left unset.
    assert_eq!(loose.budgets.daily_microusd.unwrap().get(), 100_000);
    let tight = profile.for_project(Some(r"D:\work\tight"));
    assert_eq!(tight.protected_paths, ["infra/**", "secrets/**"]);
    assert_eq!(tight.budgets.per_ticket_microusd.unwrap().get(), 100_000);
    let none = profile.for_project(None);
    assert_eq!(none, profile);
}

// Invariant: the same folder written two ways is one project, and two
// folders that share a last name are two. Grouping by the last name would
// merge unrelated projects.
#[test]
fn projects_are_keyed_by_the_full_path() {
    let source = "[projects.\"D:/a/app\"]\n[projects.\"D:/b/app\"]\n";
    let profile = Profile::parse("p.toml", source).unwrap();
    assert_eq!(profile.projects.len(), 2);
    assert_eq!(profile.project(r"D:\a\app\").unwrap().path, "D:/a/app");
    assert_eq!(profile.project("D:/b/app").unwrap().name, "app");
    assert!(profile.project("D:/c/app").is_none());
    let twice = "[projects.\"D:/a/app\"]\n[projects.'D:\\a\\app\\']\n";
    assert!(Profile::parse("p.toml", twice).is_err());
}

// Invariant: a custom pipeline can switch steps off but only into an order
// the engine can run, cannot replace a built-in, and cannot loop. A pipeline
// that kept build without plan would start work with no criteria.
#[test]
fn custom_pipelines_are_validated() {
    let ok = "[pipelines.mine]\nbased_on = \"bug-fix\"\noff = [\"clarify\"]\nrounds = 2\n\n[pipelines.next]\nbased_on = \"mine\"\nif_needed = [\"design\"]\n";
    let profile = Profile::parse("p.toml", ok).unwrap();
    let mine = profile.pipeline("mine").unwrap();
    assert!(!mine.builtin);
    assert_eq!(mine.off(), [Step::Clarify, Step::Design]);
    assert_eq!(mine.rounds, Some(2));
    assert_eq!(
        profile.pipeline("next").unwrap().mode(Step::Design),
        autopilot_profile::StepMode::IfNeeded
    );
    for bad in [
        "[pipelines.full]\noff = [\"clarify\"]\n",
        "[pipelines.a]\noff = [\"plan\"]\n",
        "[pipelines.a]\noff = [\"understand\"]\n",
        "[pipelines.a]\noff = [\"build_check\"]\n",
        "[pipelines.a]\nbased_on = \"b\"\n[pipelines.b]\nbased_on = \"a\"\n",
        "[pipelines.a]\nbased_on = \"nothing\"\n",
        "[pipelines.\"Bad Name\"]\n",
        "[pipelines.a]\nrounds = 0\n",
        "[projects.\"D:/x\"]\ndefault_pipeline = \"nope\"\n",
    ] {
        assert!(Profile::parse("p.toml", bad).is_err(), "{bad}");
    }
}

// Invariant: ask-only runs Understand and nothing that writes, and the
// built-in presets are the ones the docs name.
#[test]
fn ask_only_runs_only_understand_and_clarify() {
    let profile = Profile::parse("p.toml", "").unwrap();
    let ask = profile.pipeline("ask-only").unwrap();
    assert!(ask.builtin);
    assert_eq!(
        ask.off(),
        [
            Step::Design,
            Step::Plan,
            Step::BuildCheck,
            Step::PullRequest
        ]
    );
    assert_eq!(profile.pipeline("quick-change").unwrap().rounds, Some(2));
    assert_eq!(
        profile.pipeline("ui-change").unwrap().mode(Step::Design),
        autopilot_profile::StepMode::On
    );
}

const SECURITY: &str = "[custom_steps.custom_1]\nname = \"Security review\"\nprompt = \"Look for injection and unsafe input handling.\"\n";

fn parse(source: &str) -> Result<Profile, autopilot_profile::ProfileDiagnostic> {
    Profile::parse(Path::new("profile.toml"), source)
}

// A custom step is only useful if a pipeline can run it. It must be defined,
// run after the build, and carry the same settings as any step.
#[test]
fn a_pipeline_can_run_a_custom_step_the_profile_defines() {
    let source = format!(
        "{SECURITY}\n[steps.custom_1]\nask_after = true\n\n[pipelines.careful]\nbased_on = \"full\"\non = [\"custom_1\"]\n"
    );
    let profile = parse(&source).unwrap();
    let step = Step::Custom(1);
    assert_eq!(profile.custom_step(step).unwrap().name, "Security review");
    assert!(profile.step(step).unwrap().ask_after);
    assert_eq!(profile.pipeline("careful").unwrap().custom_steps(), [step]);
    // No built-in pipeline and no other custom one runs it.
    assert_eq!(profile.pipeline("full").unwrap().custom_steps(), []);
}

#[test]
fn a_bad_custom_step_is_refused_with_its_key() {
    for (source, key) in [
        // Defined but never named.
        (
            "[custom_steps.custom_1]\nprompt = \"x\"\n".to_owned(),
            "custom_steps.custom_1",
        ),
        (
            "[custom_steps.custom_1]\nname = \"a\"\n".to_owned(),
            "custom_steps.custom_1",
        ),
        // A built-in step cannot be redefined here.
        (
            "[custom_steps.plan]\nname = \"a\"\nprompt = \"x\"\n".to_owned(),
            "custom_steps.plan",
        ),
        // Settings for a step nobody defined.
        (
            "[steps.custom_2]\nask_after = true\n".to_owned(),
            "steps.custom_2",
        ),
        // A pipeline cannot run a step that does not exist.
        (
            "[pipelines.a]\non = [\"custom_3\"]\n".to_owned(),
            "pipelines.a",
        ),
        // Custom steps run after the build, so the build must run.
        (
            format!("{SECURITY}\n[pipelines.a]\nbased_on = \"ask-only\"\non = [\"custom_1\"]\n"),
            "pipelines.a",
        ),
        // A custom step is on or off, never "if needed".
        (
            format!("{SECURITY}\n[pipelines.a]\nif_needed = [\"custom_1\"]\n"),
            "pipelines.a",
        ),
    ] {
        let error = parse(&source).expect_err(&source);
        assert!(error.to_string().contains(key), "{source}: {error}");
    }
}

#[test]
fn the_advisor_defaults_to_a_strong_model_with_a_small_cap() {
    let profile = Profile::parse("project.toml", "").unwrap();

    assert_eq!(profile.advisor.model, "opus");
    assert_eq!(profile.advisor.max_calls, 3);
    assert_eq!(profile.advisor.chat_calls, 20);
    let error = Profile::parse("project.toml", "[advisor]\nchat_calls = 101\n").unwrap_err();
    assert!(error.message.contains("chat_calls"));
}

#[test]
fn an_advisor_cap_that_is_too_large_points_at_the_advisor_section() {
    let error = Profile::parse("project.toml", "[advisor]\nmax_calls = 99\n").unwrap_err();

    assert_eq!(error.key, "advisor");
    assert!(error.message.contains("max_calls"));
}

// Invariant: a step with no model in the profile gets a deliberate one, not
// the cheapest by accident: opus for the plan, haiku for steps that only carry
// results, sonnet for the rest. An explicit choice still wins.
#[test]
fn steps_default_to_a_model_that_fits_their_job() {
    let profile =
        Profile::parse("project.toml", "[steps.build_check]\nmodel = \"haiku\"\n").unwrap();
    let model = |step| profile.step(step).unwrap().model.clone().unwrap();

    assert_eq!(model(Step::Plan), "opus");
    assert_eq!(model(Step::Clarify), "haiku");
    assert_eq!(model(Step::Understand), "sonnet");
    assert_eq!(model(Step::BuildCheck), "haiku");
}
