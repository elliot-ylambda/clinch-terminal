use ::channel_versions::VersionInfo;

use super::*;

fn version() -> VersionInfo {
    VersionInfo::new("v2".to_owned())
}

fn step(stage: AutoupdateStage, opened_here: bool) -> Option<UpdatePopupStep> {
    update_popup_step(&stage, None, None, opened_here)
}

#[test]
fn every_update_waiting_for_the_user_asks_first_whether_or_not_it_is_downloaded() {
    for stage in [
        AutoupdateStage::UpdateAvailable {
            new_version: version(),
            update_id: "available".to_owned(),
        },
        AutoupdateStage::UpdateReady {
            new_version: version(),
            update_id: "downloaded".to_owned(),
        },
    ] {
        assert_eq!(
            step(stage, true),
            Some(UpdatePopupStep::Confirm {
                version: "v2".to_owned()
            })
        );
    }
}

#[test]
fn an_update_nobody_asked_about_here_stays_out_of_the_way() {
    let available = AutoupdateStage::UpdateAvailable {
        new_version: version(),
        update_id: "available".to_owned(),
    };
    assert_eq!(step(available, false), None);
    assert_eq!(step(AutoupdateStage::CheckingForUpdate, false), None);
    assert_eq!(step(AutoupdateStage::NoUpdateAvailable, true), None);
}

#[test]
fn an_install_under_way_shows_in_every_window_and_stays_up() {
    let installing = [
        AutoupdateStage::Updating {
            new_version: version(),
            update_id: "ready".to_owned(),
        },
        AutoupdateStage::UpdatedPendingRestart {
            new_version: version(),
        },
    ];
    for stage in installing {
        let shown = step(stage, false).unwrap();
        assert_eq!(shown, UpdatePopupStep::Installing);
        assert!(!shown.closes_on_click_outside());
    }
    assert!(!UpdatePopupStep::Downloading {
        received: 1,
        total: 2
    }
    .closes_on_click_outside());
    assert!(UpdatePopupStep::Confirm {
        version: "v2".to_owned()
    }
    .closes_on_click_outside());
}

#[test]
fn the_download_steps_follow_its_progress() {
    let downloading =
        |progress| update_popup_step(&AutoupdateStage::DownloadingUpdate, progress, None, false);
    assert_eq!(
        downloading(None),
        Some(UpdatePopupStep::Downloading {
            received: 0,
            total: 0
        })
    );
    assert_eq!(
        downloading(Some(ClinchDownloadProgress::Downloading {
            received: 5,
            total: 10
        })),
        Some(UpdatePopupStep::Downloading {
            received: 5,
            total: 10
        })
    );
    assert_eq!(
        downloading(Some(ClinchDownloadProgress::Verifying)),
        Some(UpdatePopupStep::Verifying)
    );
}

#[test]
fn a_failure_shows_until_it_is_closed_even_though_the_update_is_still_available() {
    let available = AutoupdateStage::UpdateAvailable {
        new_version: version(),
        update_id: "available".to_owned(),
    };
    assert_eq!(
        update_popup_step(&available, None, Some(ClinchUpdateFailure::Download), false),
        Some(UpdatePopupStep::Failed(ClinchUpdateFailure::Download))
    );
}

#[test]
fn the_note_says_what_restarting_does_to_running_work() {
    assert_eq!(
        restart_note(0, 0, true),
        "Clinch will quit and reopen with your tabs."
    );
    assert_eq!(
        restart_note(1, 0, true),
        "Clinch will quit and reopen with your tabs. 1 running session will continue."
    );
    assert_eq!(
        restart_note(3, 2, true),
        "Clinch will quit and reopen with your tabs. 3 running sessions will continue. \
         2 running commands will stop."
    );
    assert_eq!(
        restart_note(2, 1, false),
        "Clinch will quit and reopen with your tabs. 2 running sessions will stop and reopen. \
         1 running command will stop."
    );
}

#[test]
fn the_download_line_shows_percent_and_size() {
    assert_eq!(download_status(0, 0), None);
    assert_eq!(
        download_status(52_000_000, 123_400_000).as_deref(),
        Some("42% · 52 of 124 MB")
    );
    assert_eq!(
        download_status(123_400_000, 123_400_000).as_deref(),
        Some("100% · 123 of 124 MB")
    );
}
