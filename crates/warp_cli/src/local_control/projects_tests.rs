use clap::Parser as _;

use super::*;
use crate::local_control::selectors::target_selector;
use crate::local_control::{ControlArgs, ControlCommand, TabCommand};

#[test]
fn transfer_keeps_source_project_and_exact_destination_selectors() {
    let parsed = ControlArgs::try_parse_from([
        "clinch",
        "tab",
        "transfer",
        "--project",
        "source-id",
        "--tab",
        "tab-id",
        "--to-project",
        "destination-id",
        "--section",
        "review",
        "--index",
        "0",
    ])
    .unwrap();
    let ControlCommand::Tab(TabCommand::Transfer(args)) = parsed.command else {
        panic!("transfer")
    };
    assert_eq!(
        target_selector(&args.target).unwrap().project.as_deref(),
        Some("source-id")
    );
    assert_eq!(args.to_project, "destination-id");
    assert_eq!(args.section.as_deref(), Some("review"));
    assert_eq!(args.index, Some(0));
}

#[test]
fn layout_file_reader_rejects_oversized_and_executable_documents() {
    let path = std::env::temp_dir().join(format!("clinch-layout-{}.json", uuid::Uuid::new_v4()));
    File::create_new(&path).unwrap();
    std::fs::write(&path, vec![b' '; MAX_PROJECT_LAYOUT_BYTES + 1]).unwrap();
    assert_eq!(
        read_layout(&path).unwrap_err().code,
        ErrorCode::InvalidParams
    );
    std::fs::write(
        &path,
        r#"{"version":1,"active_tab":0,"tabs":[],"sections":[],"command":"echo unexpected"}"#,
    )
    .unwrap();
    assert_eq!(
        read_layout(&path).unwrap_err().code,
        ErrorCode::InvalidParams
    );
    std::fs::remove_file(path).unwrap();
}
