use ::channel_versions::VersionInfo;

use super::*;

fn version() -> VersionInfo {
    VersionInfo::new("v2".to_owned())
}

fn step(stage: AutoupdateStage, opened_here: bool) -> Option<UpdatePopupStep> {
    update_popup_step(&stage, None, None, None, opened_here)
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
        (
            AutoupdateStage::Updating {
                new_version: version(),
                update_id: "ready".to_owned(),
            },
            InstallStep::SavingSessions,
        ),
        (
            AutoupdateStage::UpdatedPendingRestart {
                new_version: version(),
            },
            InstallStep::Restarting,
        ),
    ];
    for (stage, install_step) in installing {
        let shown = step(stage, false).unwrap();
        assert_eq!(shown, UpdatePopupStep::Installing(install_step));
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
    let downloading = |progress| {
        update_popup_step(
            &AutoupdateStage::DownloadingUpdate,
            progress,
            None,
            None,
            false,
        )
    };
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
fn the_install_bar_follows_the_helper_and_is_only_full_once_clinch_is_quitting() {
    let updating = AutoupdateStage::Updating {
        new_version: version(),
        update_id: "ready".to_owned(),
    };
    let installing =
        |stage: &AutoupdateStage, phase| match update_popup_step(stage, None, phase, None, false) {
            Some(UpdatePopupStep::Installing(install_step)) => install_step,
            other => panic!("expected an install step, got {other:?}"),
        };
    assert_eq!(installing(&updating, None), InstallStep::SavingSessions);
    assert_eq!(
        installing(&updating, Some(ClinchInstallPhase::SavingSessions)),
        InstallStep::SavingSessions
    );
    assert_eq!(
        installing(&updating, Some(ClinchInstallPhase::PreparingApp)),
        InstallStep::PreparingApp
    );
    let restarting = AutoupdateStage::UpdatedPendingRestart {
        new_version: version(),
    };
    assert_eq!(
        installing(&restarting, Some(ClinchInstallPhase::PreparingApp)),
        InstallStep::Restarting
    );

    // A full bar while Clinch is still working on the install reads as frozen.
    let steps = [
        InstallStep::SavingSessions,
        InstallStep::PreparingApp,
        InstallStep::Restarting,
    ];
    let fractions = steps.map(InstallStep::fraction);
    assert!(
        fractions.windows(2).all(|pair| pair[0] < pair[1]),
        "{fractions:?}"
    );
    assert!(fractions[0] > 0. && fractions[1] < 1.);
    assert_eq!(fractions[2], 1.);
    assert_eq!(
        steps.map(InstallStep::description),
        [
            "Saving your sessions (step 1 of 3). Clinch will quit and reopen on its own.",
            "Getting the new version ready (step 2 of 3). Clinch will quit and reopen on its own.",
            "Quitting to finish (step 3 of 3). Clinch will reopen on its own.",
        ]
    );
}

#[test]
fn a_failure_shows_until_it_is_closed_even_though_the_update_is_still_available() {
    let available = AutoupdateStage::UpdateAvailable {
        new_version: version(),
        update_id: "available".to_owned(),
    };
    assert_eq!(
        update_popup_step(
            &available,
            None,
            None,
            Some(ClinchUpdateFailure::Download),
            false
        ),
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
