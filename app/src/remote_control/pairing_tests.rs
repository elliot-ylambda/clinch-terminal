use ring::rand::SystemRandom;
use ring::signature::KeyPair as _;

use super::*;

fn valid_public_key() -> String {
    let mut key = [0_u8; 65];
    key[0] = 4;
    BASE64_STANDARD.encode(key)
}

fn claim_request(invitation: &PairingInvitation) -> PairingClaimRequest {
    let fragment = invitation.pairing_url.split('#').nth(1).unwrap();
    let (_, secret) = fragment.split_once(':').unwrap();
    PairingClaimRequest {
        invitation_id: invitation.id,
        secret: secret.to_owned(),
        device_name: "Elliot's iPhone".to_owned(),
        platform: DevicePlatform::Ios,
        public_key_p256_raw: valid_public_key(),
    }
}

fn authenticate_device(
    manager: &PairingManager,
    device_id: DeviceId,
    key_pair: &signature::EcdsaKeyPair,
    rng: &SystemRandom,
    now: DateTime<Utc>,
) -> AuthenticatedSession {
    let challenge = manager
        .create_challenge(AuthChallengeRequest { device_id }, now)
        .unwrap();
    let challenge_bytes = BASE64_STANDARD.decode(&challenge.challenge).unwrap();
    let signed = key_pair.sign(rng, &challenge_bytes).unwrap();
    manager
        .authenticate(
            Authenticate {
                device_id,
                challenge_id: challenge.id,
                signature: BASE64_STANDARD.encode(signed.as_ref()),
                last_seen_sequence: 0,
            },
            now,
        )
        .unwrap()
}

fn signing_key(rng: &SystemRandom) -> signature::EcdsaKeyPair {
    let pkcs8 =
        signature::EcdsaKeyPair::generate_pkcs8(&signature::ECDSA_P256_SHA256_FIXED_SIGNING, rng)
            .unwrap();
    signature::EcdsaKeyPair::from_pkcs8(
        &signature::ECDSA_P256_SHA256_FIXED_SIGNING,
        pkcs8.as_ref(),
        rng,
    )
    .unwrap()
}

fn manager_for_key(
    key_pair: &signature::EcdsaKeyPair,
    now: DateTime<Utc>,
) -> (PairingManager, DeviceId) {
    let device_id = DeviceId::new();
    let manager = PairingManager::new(DeviceRegistry {
        version: REGISTRY_VERSION,
        route_id: random_route_id(),
        devices: vec![PairedDeviceRecord {
            id: device_id,
            name: "Phone".to_owned(),
            platform: DevicePlatform::Ios,
            capabilities: vec![Capability::View, Capability::Control],
            public_key_p256_raw: BASE64_STANDARD.encode(key_pair.public_key().as_ref()),
            paired_at: now,
            last_seen_at: Some(now),
            revoked_at: None,
        }],
    })
    .unwrap();
    (manager, device_id)
}

#[test]
fn invitation_is_single_use_and_requires_desktop_approval() {
    let manager = PairingManager::new(DeviceRegistry::default()).unwrap();
    let now = Utc::now();
    let invitation = manager
        .create_invitation("https://mac.example.ts.net", now)
        .unwrap();
    let request = claim_request(&invitation);
    let receipt = manager.claim(request.clone(), now).unwrap();

    assert_eq!(manager.claim(request, now), Err(PairingError::NotFound));
    assert_eq!(
        manager
            .pairing_status(
                PairingStatusRequest {
                    claim_id: receipt.claim_id,
                    claim_secret: receipt.claim_secret.clone(),
                },
                now,
            )
            .unwrap(),
        PairingStatus::Pending
    );

    let record = manager
        .approve(
            receipt.claim_id,
            vec![Capability::View, Capability::Control],
            now,
        )
        .unwrap();
    assert!(matches!(
        manager
            .pairing_status(
                PairingStatusRequest {
                    claim_id: receipt.claim_id,
                    claim_secret: receipt.claim_secret,
                },
                now,
            )
            .unwrap(),
        PairingStatus::Approved { device_id, .. } if device_id == record.id
    ));
}

#[test]
fn rescanning_the_same_phone_key_reuses_its_device_record() {
    let manager = PairingManager::new(DeviceRegistry::default()).unwrap();
    let now = Utc::now();
    let first_invitation = manager
        .create_invitation("https://mac.example.ts.net", now)
        .unwrap();
    let first_receipt = manager
        .claim(claim_request(&first_invitation), now)
        .unwrap();
    let first = manager
        .approve(first_receipt.claim_id, vec![Capability::View], now)
        .unwrap();

    let retried_at = now + Duration::seconds(1);
    let second_invitation = manager
        .create_invitation("https://mac.example.ts.net", retried_at)
        .unwrap();
    let second_receipt = manager
        .claim(claim_request(&second_invitation), retried_at)
        .unwrap();
    let second = manager
        .approve(
            second_receipt.claim_id,
            vec![Capability::View, Capability::Control],
            retried_at,
        )
        .unwrap();

    assert_eq!(second.id, first.id);
    assert_eq!(second.paired_at, retried_at);
    assert_eq!(
        second.capabilities,
        vec![Capability::View, Capability::Control]
    );
    assert_eq!(manager.registry_snapshot().unwrap().devices.len(), 1);
    assert!(matches!(
        manager
            .pairing_status(
                PairingStatusRequest {
                    claim_id: second_receipt.claim_id,
                    claim_secret: second_receipt.claim_secret,
                },
                retried_at,
            )
            .unwrap(),
        PairingStatus::Approved { device_id, .. } if device_id == first.id
    ));
}

#[test]
fn valid_approval_polling_does_not_consume_the_security_attempt_budget() {
    let manager = PairingManager::new(DeviceRegistry::default()).unwrap();
    let now = Utc::now();
    let invitation = manager
        .create_invitation("https://mac.example.ts.net", now)
        .unwrap();
    let receipt = manager.claim(claim_request(&invitation), now).unwrap();
    let request = PairingStatusRequest {
        claim_id: receipt.claim_id,
        claim_secret: receipt.claim_secret,
    };

    for _ in 0..(MAX_SECURITY_ATTEMPTS_PER_MINUTE * 2) {
        assert_eq!(
            manager.pairing_status(request.clone(), now).unwrap(),
            PairingStatus::Pending
        );
    }
}

#[test]
fn registry_validation_keeps_the_newest_record_for_a_phone_key() {
    let now = Utc::now();
    let older_id = DeviceId::new();
    let newer_id = DeviceId::new();
    let record = |id, paired_at| PairedDeviceRecord {
        id,
        name: "Phone".to_owned(),
        platform: DevicePlatform::Ios,
        capabilities: vec![Capability::View],
        public_key_p256_raw: valid_public_key(),
        paired_at,
        last_seen_at: None,
        revoked_at: None,
    };
    let registry = DeviceRegistry {
        version: REGISTRY_VERSION,
        route_id: random_route_id(),
        devices: vec![
            record(older_id, now),
            record(newer_id, now + Duration::seconds(1)),
        ],
    }
    .validate()
    .unwrap();

    assert_eq!(registry.devices.len(), 1);
    assert_eq!(registry.devices[0].id, newer_id);
}

#[test]
fn wrong_invitation_secret_consumes_the_invitation() {
    let manager = PairingManager::new(DeviceRegistry::default()).unwrap();
    let now = Utc::now();
    let invitation = manager
        .create_invitation("https://mac.example.ts.net", now)
        .unwrap();
    let mut request = claim_request(&invitation);
    request.secret = random_secret();

    assert_eq!(manager.claim(request, now), Err(PairingError::Unauthorized));
    assert_eq!(
        manager.claim(claim_request(&invitation), now),
        Err(PairingError::NotFound)
    );
}

#[test]
fn cancelled_and_expired_invitations_cannot_be_claimed() {
    let manager = PairingManager::new(DeviceRegistry::default()).unwrap();
    let now = Utc::now();
    let cancelled = manager
        .create_invitation("https://mac.example.ts.net", now)
        .unwrap();
    manager.cancel_invitation(cancelled.id).unwrap();
    assert_eq!(
        manager.claim(claim_request(&cancelled), now),
        Err(PairingError::NotFound)
    );

    let expiring = manager
        .create_invitation("https://mac.example.ts.net", now)
        .unwrap();
    assert_eq!(
        manager.claim(
            claim_request(&expiring),
            now + Duration::seconds(PAIRING_INVITATION_TTL_SECS as i64 + 1),
        ),
        Err(PairingError::NotFound)
    );
}

#[test]
fn wrong_signature_consumes_the_challenge_without_authorizing() {
    let now = Utc::now();
    let rng = SystemRandom::new();
    let authorized_key = signing_key(&rng);
    let wrong_key = signing_key(&rng);
    let (manager, device_id) = manager_for_key(&authorized_key, now);
    let challenge = manager
        .create_challenge(AuthChallengeRequest { device_id }, now)
        .unwrap();
    let challenge_bytes = BASE64_STANDARD.decode(&challenge.challenge).unwrap();
    let wrong_signature = wrong_key.sign(&rng, &challenge_bytes).unwrap();

    let request = Authenticate {
        device_id,
        challenge_id: challenge.id,
        signature: BASE64_STANDARD.encode(wrong_signature.as_ref()),
        last_seen_sequence: 0,
    };
    assert!(matches!(
        manager.authenticate(request.clone(), now),
        Err(PairingError::Unauthorized)
    ));
    assert!(matches!(
        manager.authenticate(request, now),
        Err(PairingError::NotFound)
    ));
}

#[test]
fn inactive_devices_and_expired_sessions_are_rejected() {
    let now = Utc::now();
    let rng = SystemRandom::new();
    let key_pair = signing_key(&rng);
    let inactive_at = now + Duration::days(DEVICE_INACTIVITY_LIMIT_DAYS + 1);
    let (inactive_manager, inactive_device_id) = manager_for_key(&key_pair, now);
    assert_eq!(
        inactive_manager.create_challenge(
            AuthChallengeRequest {
                device_id: inactive_device_id,
            },
            inactive_at,
        ),
        Err(PairingError::Expired)
    );

    let (manager, device_id) = manager_for_key(&key_pair, now);
    let session = authenticate_device(&manager, device_id, &key_pair, &rng, now);
    assert!(matches!(
        manager.authorize_session(
            &session.cookie_token,
            now + Duration::seconds(AUTH_SESSION_TTL_SECS as i64 + 1),
        ),
        Err(PairingError::Unauthorized)
    ));
}

#[test]
fn revoke_all_clears_every_ephemeral_authorization() {
    let now = Utc::now();
    let rng = SystemRandom::new();
    let key_pair = signing_key(&rng);
    let (manager, device_id) = manager_for_key(&key_pair, now);
    let session = authenticate_device(&manager, device_id, &key_pair, &rng, now);
    manager.revoke_all_devices(now).unwrap();

    assert!(manager.paired_devices(now).unwrap().is_empty());
    assert!(matches!(
        manager.authorize_session(&session.cookie_token, now),
        Err(PairingError::Unauthorized)
    ));
    assert_eq!(
        manager.create_challenge(AuthChallengeRequest { device_id }, now),
        Err(PairingError::Revoked)
    );
}

#[test]
fn revocation_invalidates_sessions_and_challenges() {
    let now = Utc::now();
    let device_id = DeviceId::new();
    let registry = DeviceRegistry {
        version: REGISTRY_VERSION,
        route_id: random_route_id(),
        devices: vec![PairedDeviceRecord {
            id: device_id,
            name: "Phone".to_owned(),
            platform: DevicePlatform::Ios,
            capabilities: vec![Capability::View],
            public_key_p256_raw: valid_public_key(),
            paired_at: now,
            last_seen_at: Some(now),
            revoked_at: None,
        }],
    };
    let manager = PairingManager::new(registry).unwrap();
    manager
        .create_challenge(AuthChallengeRequest { device_id }, now)
        .unwrap();
    manager.revoke_device(device_id, now).unwrap();

    assert_eq!(
        manager.create_challenge(AuthChallengeRequest { device_id }, now),
        Err(PairingError::Revoked)
    );
}

#[test]
fn connection_limit_counts_only_claimed_websocket_sessions() {
    let now = Utc::now();
    let rng = SystemRandom::new();
    let key_pair = signing_key(&rng);
    let (manager, device_id) = manager_for_key(&key_pair, now);

    // A completed HTTP authentication is not a connection until the WebSocket claims it.
    let abandoned = authenticate_device(&manager, device_id, &key_pair, &rng, now);
    let replacement = authenticate_device(&manager, device_id, &key_pair, &rng, now);
    assert!(matches!(
        manager.authorize_session(&abandoned.cookie_token, now),
        Err(PairingError::Unauthorized)
    ));

    manager
        .connect_session(&replacement.cookie_token, now)
        .unwrap();
    let mut connected = vec![replacement];
    while connected.len() < MAX_CONNECTIONS_PER_DEVICE {
        let session = authenticate_device(&manager, device_id, &key_pair, &rng, now);
        manager.connect_session(&session.cookie_token, now).unwrap();
        connected.push(session);
    }
    assert!(matches!(
        manager.connect_session(&connected[0].cookie_token, now),
        Err(PairingError::AlreadyUsed)
    ));
    assert!(manager.paired_devices(now).unwrap()[0].connected);

    let challenge = manager
        .create_challenge(AuthChallengeRequest { device_id }, now)
        .unwrap();
    let challenge_bytes = BASE64_STANDARD.decode(&challenge.challenge).unwrap();
    let signed = key_pair.sign(&rng, &challenge_bytes).unwrap();
    assert!(matches!(
        manager.authenticate(
            Authenticate {
                device_id,
                challenge_id: challenge.id,
                signature: BASE64_STANDARD.encode(signed.as_ref()),
                last_seen_sequence: 0,
            },
            now,
        ),
        Err(PairingError::Capacity)
    ));

    for session in connected {
        manager.end_session(session.response.session_id).unwrap();
    }
    assert!(!manager.paired_devices(now).unwrap()[0].connected);
}
