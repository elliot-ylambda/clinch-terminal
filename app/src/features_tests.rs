use super::*;

#[test]
fn backend_free_channels_disable_inherited_ssh_extension() {
    for is_release_bundle in [false, true] {
        let flags = enabled_features_for_channel(
            DOGFOOD_FLAGS.iter().copied().collect(),
            is_release_bundle,
            false,
        );
        assert!(!flags.contains(&FeatureFlag::SshRemoteServer));
        for &flag in DOGFOOD_FLAGS {
            if flag != FeatureFlag::SshRemoteServer {
                assert!(flags.contains(&flag));
            }
        }
    }
}

#[test]
fn backend_enabled_channels_preserve_ssh_extension() {
    for (additional_features, is_release_bundle) in [
        (DOGFOOD_FLAGS.iter().copied().collect(), false),
        (HashSet::new(), true),
    ] {
        let flags = enabled_features_for_channel(additional_features, is_release_bundle, true);
        assert!(flags.contains(&FeatureFlag::SshRemoteServer));
    }
}
