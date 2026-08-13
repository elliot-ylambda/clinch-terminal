use super::*;

fn bundled_skill_contents() -> String {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../resources/bundled/agent-skills/clinch-toolbelt/SKILL.md"
    );
    std::fs::read_to_string(path).expect("bundled clinch-toolbelt SKILL.md must exist")
}

fn bundled_control_skill_contents() -> String {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../resources/bundled/agent-skills/clinch-control/SKILL.md"
    );
    std::fs::read_to_string(path).expect("bundled clinch-control SKILL.md must exist")
}

#[test]
fn bundled_skill_carries_a_managed_marker() {
    let contents = bundled_skill_contents();
    assert!(
        contents.contains("<!-- managed-by: Clinch; version: 2.2.0 -->"),
        "the bundled skill must carry the Clinch managed marker"
    );
}

#[test]
fn bundled_control_skill_routes_only_persistent_processes_to_new_tabs() {
    let contents = bundled_control_skill_contents();
    assert!(contents.contains("<!-- managed-by: Clinch; version: 1.3.0 -->"));
    assert!(contents.contains("tab create"));
    assert!(contents.contains("--cwd"));
    assert!(contents.contains("dev server"));
    assert!(contents.contains("tests, linters"));
    assert!(contents.contains("Do not create a tab merely because a command is a subprocess"));
    assert!(contents.contains("CLINCH_CONTROL_COMMAND"));
    assert!(contents.contains("CLINCH_CONTROL_WRAPPER"));
    assert!(contents.contains("CLINCH_CONTROL_PID"));
    assert!(contents.contains("--pid \"$CLINCH_CONTROL_PID\""));
    assert!(contents.contains("WARP_FOCUS_URL"));
}

#[test]
fn bundled_control_skills_are_channel_neutral() {
    for contents in [bundled_control_skill_contents(), bundled_skill_contents()] {
        assert!(contents.contains("CLINCH_CONTROL_WRAPPER"));
        assert!(contents.contains("CLINCH_CONTROL_PID"));
        assert!(contents.contains("WARP_FOCUS_URL"));
        assert!(!contents.contains("{{clinch_control_binary_name}}"));
        assert!(!contents.contains("{{clinch_control_wrapper_path}}"));
        assert!(!contents.contains("/Applications/Clinch"));
    }
}

#[test]
fn bundled_skill_uses_typed_toolbelt_control_without_editing_persistence() {
    let contents = bundled_skill_contents();
    assert!(contents.contains("toolbelt list"));
    assert!(contents.contains("toolbelt button create"));
    assert!(contents.contains("toolbelt button delete"));
    assert!(contents.contains("toolbelt button move"));
    assert!(contents.contains("CLINCH_CONTROL_COMMAND"));
    assert!(contents.contains("CLINCH_CONTROL_WRAPPER"));
    assert!(contents.contains("CLINCH_CONTROL_PID"));
    assert!(contents.contains("--pid \"$CLINCH_CONTROL_PID\""));
    assert!(contents.contains("Never edit `settings.toml`, SQLite"));
    assert!(!contents.contains("[agents.third_party"));
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
fn codex_presence_installs_skills_into_the_shared_agent_skills_location() {
    let source = scratch_bundle(MANAGED_V1);
    let home = tempfile::tempdir().unwrap();
    let codex_home = home.path().join(".codex");
    let agent_skills_root = home.path().join(".agents");
    std::fs::create_dir_all(&codex_home).unwrap();

    install_skills_for_agent(source.path(), &codex_home, &agent_skills_root);

    let installed = agent_skills_root.join("skills/clinch-toolbelt/SKILL.md");
    assert_eq!(std::fs::read_to_string(installed).unwrap(), MANAGED_V1);
}

#[test]
fn codex_migration_removes_only_replaced_managed_legacy_skill() {
    let source = scratch_bundle(MANAGED_V2);
    let home = tempfile::tempdir().unwrap();
    let codex_home = home.path().join(".codex");
    let agent_skills_root = home.path().join(".agents");
    let legacy_dir = codex_home.join("skills/clinch-toolbelt");
    std::fs::create_dir_all(&legacy_dir).unwrap();
    std::fs::write(legacy_dir.join("SKILL.md"), MANAGED_V1).unwrap();

    install_skills_for_agent(source.path(), &codex_home, &agent_skills_root);
    remove_migrated_legacy_skills(
        source.path(),
        &codex_home.join("skills"),
        &agent_skills_root,
    );

    assert!(!legacy_dir.exists());
    assert_eq!(
        std::fs::read_to_string(
            agent_skills_root
                .join("skills/clinch-toolbelt")
                .join("SKILL.md")
        )
        .unwrap(),
        MANAGED_V2
    );
}

#[test]
fn codex_migration_preserves_user_owned_legacy_skill() {
    let source = scratch_bundle(MANAGED_V2);
    let home = tempfile::tempdir().unwrap();
    let codex_home = home.path().join(".codex");
    let agent_skills_root = home.path().join(".agents");
    let legacy = codex_home.join("skills/clinch-toolbelt/SKILL.md");
    std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    std::fs::write(&legacy, UNMANAGED).unwrap();

    install_skills_for_agent(source.path(), &codex_home, &agent_skills_root);
    remove_migrated_legacy_skills(
        source.path(),
        &codex_home.join("skills"),
        &agent_skills_root,
    );

    assert_eq!(std::fs::read_to_string(legacy).unwrap(), UNMANAGED);
}

#[test]
fn codex_migration_preserves_extra_legacy_directory_files() {
    let source = scratch_bundle(MANAGED_V2);
    let home = tempfile::tempdir().unwrap();
    let codex_home = home.path().join(".codex");
    let agent_skills_root = home.path().join(".agents");
    let legacy_dir = codex_home.join("skills/clinch-toolbelt");
    std::fs::create_dir_all(&legacy_dir).unwrap();
    std::fs::write(legacy_dir.join("SKILL.md"), MANAGED_V1).unwrap();
    std::fs::write(legacy_dir.join("notes.md"), "user notes").unwrap();

    install_skills_for_agent(source.path(), &codex_home, &agent_skills_root);
    remove_migrated_legacy_skills(
        source.path(),
        &codex_home.join("skills"),
        &agent_skills_root,
    );

    assert!(!legacy_dir.join("SKILL.md").exists());
    assert_eq!(
        std::fs::read_to_string(legacy_dir.join("notes.md")).unwrap(),
        "user notes"
    );
}

#[test]
fn provisioning_is_gated_to_clinch_app_ids() {
    assert!(app_id_enables_bundled_skills("sh.clinch.Clinch"));
    assert!(app_id_enables_bundled_skills("sh.clinch.ClinchDev"));
    assert!(!app_id_enables_bundled_skills("dev.warp.Warp-Stable"));
}

#[test]
#[cfg(unix)]
fn provisioning_requires_the_current_bundle_control_wrapper() {
    let resources = tempfile::tempdir().unwrap();
    assert!(!bundle_has_control_wrapper(resources.path()));

    let wrapper = bundled_control_wrapper(resources.path());
    std::fs::create_dir_all(wrapper.parent().unwrap()).unwrap();
    std::fs::write(&wrapper, "#!/bin/sh\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    assert!(bundle_has_control_wrapper(resources.path()));
}

#[test]
fn clinch_build_entrypoints_ship_the_control_wrapper() {
    let run = include_str!("../../script/run");
    let clinch_dev = include_str!("../../script/clinch-dev");
    let macos_bundle = include_str!("../../script/macos/bundle");
    let release_verifier = include_str!("../../script/verify-clinch-release");

    assert!(run.contains("FEATURES=\"gui,warp_control_cli\""));
    assert!(clinch_dev.contains("export FEATURES=\"${CLINCH_DEV_FEATURES:-gui},warp_control_cli\""));
    assert!(macos_bundle.contains("agent_mode_debug,warp_control_cli"));
    assert!(macos_bundle.contains("Stable Clinch must ship the same current-app control surface"));
    assert!(release_verifier.contains("Contents/Resources/bin/warpctrl"));
    assert!(release_verifier.contains("agent-skills/clinch-control/SKILL.md"));
}
