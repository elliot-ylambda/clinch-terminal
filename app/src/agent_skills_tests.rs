use super::*;

fn bundled_skill_contents() -> String {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../resources/bundled/agent-skills/clinch-toolbelt/SKILL.md"
    );
    std::fs::read_to_string(path).expect("bundled clinch-toolbelt SKILL.md must exist")
}

#[test]
fn bundled_skill_carries_a_managed_marker() {
    let contents = bundled_skill_contents();
    assert!(
        contents.contains("<!-- managed-by: Clinch; version: 1.1.0 -->"),
        "the bundled skill must carry the Clinch managed marker"
    );
}

/// The skill tells agents to materialize the shipped defaults before switching a
/// footer from `default` to `custom`. If the defaults change in code but not in
/// the skill, agents would delete buttons from user footers. Keep them in sync.
#[test]
fn bundled_skill_lists_every_default_custom_insert() {
    use crate::ai::blocklist::agent_view::toolbar_item::AgentToolbarItemKind;

    let contents = bundled_skill_contents();
    let defaults = AgentToolbarItemKind::cli_default_left()
        .into_iter()
        .chain(AgentToolbarItemKind::terminal_default_left());
    for item in defaults {
        if let AgentToolbarItemKind::CustomInsert { label, text } = item {
            assert!(
                contents.contains(&format!("label = \"{label}\"")),
                "SKILL.md is missing default button label {label:?}"
            );
            assert!(
                contents.contains(&format!("text = \"{text}\"")),
                "SKILL.md is missing default button text {text:?}"
            );
        }
    }
    // Spot-check the non-custom CLI defaults agents must materialize as well.
    for name in [
        "\"fork_session\"",
        "\"compact\"",
        "\"continue_prompt\"",
        "\"looks_good_prompt\"",
        "\"transfer_agent\"",
        "\"voice_input\"",
    ] {
        assert!(
            contents.contains(name),
            "SKILL.md is missing default item {name}"
        );
    }
    assert!(
        !contents.contains("/codex-build"),
        "personal /codex-build skill must not be shipped as a default button"
    );
}

const MANAGED_V1: &str = "---\nname: x\n---\n\n<!-- managed-by: Clinch; version: 1.0.0 -->\nbody";
const MANAGED_V2: &str = "---\nname: x\n---\n\n<!-- managed-by: Clinch; version: 1.1.0 -->\nbody";
const UNMANAGED: &str = "---\nname: x\n---\n\nuser wrote this";

#[test]
fn managed_version_parses_the_marker_line() {
    assert_eq!(managed_version(MANAGED_V1), Some(vec![1, 0, 0]));
    assert_eq!(managed_version(UNMANAGED), None);
    assert_eq!(
        managed_version("<!-- managed-by: Clinch; version: nonsense -->"),
        None
    );
}

#[test]
fn decide_installs_when_target_is_missing() {
    assert_eq!(decide(MANAGED_V1, None), InstallDecision::Install);
}

#[test]
fn decide_upgrades_older_managed_copies() {
    assert_eq!(
        decide(MANAGED_V2, Some(MANAGED_V1)),
        InstallDecision::Install
    );
}

#[test]
fn decide_skips_same_or_newer_managed_copies() {
    assert_eq!(
        decide(MANAGED_V1, Some(MANAGED_V1)),
        InstallDecision::SkipUpToDate
    );
    assert_eq!(
        decide(MANAGED_V1, Some(MANAGED_V2)),
        InstallDecision::SkipUpToDate
    );
}

#[test]
fn decide_never_touches_user_owned_files() {
    assert_eq!(
        decide(MANAGED_V1, Some(UNMANAGED)),
        InstallDecision::SkipUserOwned
    );
    // A bundled file without a marker is a packaging bug; refuse to overwrite anything.
    assert_eq!(
        decide(UNMANAGED, Some(MANAGED_V1)),
        InstallDecision::SkipUserOwned
    );
    assert_eq!(decide(UNMANAGED, None), InstallDecision::Install);
}

fn scratch_bundle(skill_body: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let skill_dir = dir.path().join("clinch-toolbelt");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(skill_dir.join("SKILL.md"), skill_body).unwrap();
    dir
}

#[test]
fn install_creates_the_skill_for_an_existing_agent_dir() {
    let source = scratch_bundle(MANAGED_V1);
    let agent_home = tempfile::tempdir().unwrap();
    install_skills_from(source.path(), agent_home.path());
    let installed = agent_home.path().join("skills/clinch-toolbelt/SKILL.md");
    assert_eq!(std::fs::read_to_string(installed).unwrap(), MANAGED_V1);
}

#[test]
fn install_upgrades_an_older_managed_copy() {
    let source = scratch_bundle(MANAGED_V2);
    let agent_home = tempfile::tempdir().unwrap();
    let target = agent_home.path().join("skills/clinch-toolbelt");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join("SKILL.md"), MANAGED_V1).unwrap();
    install_skills_from(source.path(), agent_home.path());
    assert_eq!(
        std::fs::read_to_string(target.join("SKILL.md")).unwrap(),
        MANAGED_V2
    );
}

#[test]
fn install_leaves_user_owned_files_alone() {
    let source = scratch_bundle(MANAGED_V2);
    let agent_home = tempfile::tempdir().unwrap();
    let target = agent_home.path().join("skills/clinch-toolbelt");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join("SKILL.md"), UNMANAGED).unwrap();
    install_skills_from(source.path(), agent_home.path());
    assert_eq!(
        std::fs::read_to_string(target.join("SKILL.md")).unwrap(),
        UNMANAGED
    );
}

#[test]
fn install_skips_agents_that_are_not_installed() {
    let source = scratch_bundle(MANAGED_V1);
    let parent = tempfile::tempdir().unwrap();
    let missing_agent_home = parent.path().join("no-such-agent");
    install_skills_from(source.path(), &missing_agent_home);
    assert!(
        !missing_agent_home.exists(),
        "must not create agent config dirs"
    );
}

#[test]
fn provisioning_is_gated_to_clinch_app_ids() {
    assert!(app_id_enables_bundled_skills("sh.clinch.Clinch"));
    assert!(app_id_enables_bundled_skills("sh.clinch.ClinchDev"));
    assert!(!app_id_enables_bundled_skills("dev.warp.Warp-Stable"));
}
