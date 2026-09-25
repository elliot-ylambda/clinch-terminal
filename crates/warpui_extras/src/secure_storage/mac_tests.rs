use super::{update_or_add, Error, ERR_SEC_ITEM_NOT_FOUND};

#[test]
fn dropping_noninteractive_storage_never_reenables_prompts() {
    use security_framework::os::macos::keychain::SecKeychain;

    // This changes only this test process's interaction policy. It does not
    // open a Keychain, read credentials, or change any item's access controls.
    let first = super::SecureStorage::new_noninteractive("test");
    first.check_access().unwrap();
    let second = super::SecureStorage::new_noninteractive("test");
    second.check_access().unwrap();
    assert!(!SecKeychain::user_interaction_allowed().unwrap());
    drop(first);
    assert!(!SecKeychain::user_interaction_allowed().unwrap());
    drop(second);
    assert!(!SecKeychain::user_interaction_allowed().unwrap());
}

#[test]
fn failure_to_disable_prompts_blocks_reads_writes_and_removals() {
    use super::super::SecureStorage as _;

    // Inject a failed setup instead of accessing the machine's Keychain.
    let storage = super::SecureStorage {
        service_name: "test".into(),
        initialization_error: Some("interaction policy unavailable".into()),
    };
    assert!(matches!(storage.read_value("key"), Err(Error::Unknown(_))));
    assert!(matches!(
        storage.write_value("key", "value"),
        Err(Error::Unknown(_))
    ));
    assert!(matches!(
        storage.remove_value("key"),
        Err(Error::Unknown(_))
    ));
}

#[test]
fn only_missing_items_are_reported_as_not_found() {
    assert!(matches!(
        Error::from(security_framework::base::Error::from_code(
            ERR_SEC_ITEM_NOT_FOUND
        )),
        Error::NotFound
    ));
    // errSecUserCanceled, errSecAuthFailed, and errSecInteractionNotAllowed.
    for code in [-128, -25293, -25308] {
        assert!(matches!(
            Error::from(security_framework::base::Error::from_code(code)),
            Error::Unknown(_)
        ));
    }
}

#[test]
fn failed_authorization_never_attempts_a_replacement_write() {
    for code in [-128, -25293, -25308] {
        let result = update_or_add::<()>(
            Err(security_framework::base::Error::from_code(code)),
            |_| panic!("cannot update an unreadable item"),
            || panic!("cannot replace an unreadable item"),
        );
        assert!(matches!(result, Err(Error::Unknown(_))));
    }
}

#[test]
fn missing_items_are_added_and_existing_items_are_updated() {
    update_or_add::<()>(
        Err(security_framework::base::Error::from_code(
            ERR_SEC_ITEM_NOT_FOUND,
        )),
        |_| panic!("missing item cannot be updated"),
        || Ok(()),
    )
    .unwrap();
    update_or_add(
        Ok(()),
        |_| Ok(()),
        || panic!("existing item cannot be replaced"),
    )
    .unwrap();
}
