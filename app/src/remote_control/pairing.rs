//! In-memory pairing/authentication authority plus the versioned registry persisted by Clinch.
//!
//! Invitation secrets, challenges, session cookies, and failed-attempt state intentionally never
//! leave this process. Only approved device public keys and metadata are serializable.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use base64::engine::general_purpose::{STANDARD as BASE64_STANDARD, URL_SAFE_NO_PAD};
use base64::Engine as _;
use chrono::{DateTime, Duration, Utc};
use clinch_companion_protocol::{
    AuthChallenge, AuthChallengeRequest, AuthSessionId, Authenticate, Authenticated, Capability,
    ChallengeId, DeviceId, DevicePlatform, DeviceSummary, PairingClaimId, PairingClaimReceipt,
    PairingClaimRequest, PairingInvitation, PairingInvitationId, PairingStatus,
    PairingStatusRequest, AUTH_CHALLENGE_TTL_SECS, AUTH_SESSION_TTL_SECS,
    DEVICE_INACTIVITY_LIMIT_DAYS, MAX_CONNECTIONS_PER_DEVICE, PAIRING_INVITATION_TTL_SECS,
};
use rand::{rngs::OsRng, RngCore as _};
use ring::signature;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use subtle::ConstantTimeEq as _;
use thiserror::Error;

const REGISTRY_VERSION: u16 = 1;
const MAX_PAIRED_DEVICES: usize = 32;
const MAX_PENDING_CLAIMS: usize = 16;
const MAX_CHALLENGES: usize = 64;
const MAX_SECURITY_ATTEMPTS_PER_MINUTE: usize = 30;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DeviceRegistry {
    pub version: u16,
    pub route_id: String,
    pub devices: Vec<PairedDeviceRecord>,
}

impl Default for DeviceRegistry {
    fn default() -> Self {
        Self {
            version: REGISTRY_VERSION,
            route_id: random_route_id(),
            devices: Vec::new(),
        }
    }
}

impl DeviceRegistry {
    pub fn validate(mut self) -> Result<Self, PairingError> {
        if self.version != REGISTRY_VERSION {
            return Err(PairingError::UnsupportedRegistryVersion(self.version));
        }
        if self.route_id.len() != 24
            || !self
                .route_id
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        {
            return Err(PairingError::InvalidRegistry);
        }
        if self.devices.len() > MAX_PAIRED_DEVICES {
            return Err(PairingError::InvalidRegistry);
        }

        self.devices.sort_by_key(|device| device.paired_at);
        self.devices.dedup_by_key(|device| device.id);
        for device in &self.devices {
            device.validate()?;
        }
        Ok(self)
    }

    pub fn route_path(&self) -> String {
        format!("/clinch-remote-{}", self.route_id)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PairedDeviceRecord {
    pub id: DeviceId,
    pub name: String,
    pub platform: DevicePlatform,
    pub capabilities: Vec<Capability>,
    pub public_key_p256_raw: String,
    pub paired_at: DateTime<Utc>,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
}

impl PairedDeviceRecord {
    fn validate(&self) -> Result<(), PairingError> {
        let key = BASE64_STANDARD
            .decode(&self.public_key_p256_raw)
            .map_err(|_| PairingError::InvalidRegistry)?;
        if self.name.is_empty()
            || self.name.len() > clinch_companion_protocol::MAX_DEVICE_NAME_BYTES
            || self.name.chars().any(char::is_control)
            || key.len() != 65
            || key.first() != Some(&4)
        {
            return Err(PairingError::InvalidRegistry);
        }
        Ok(())
    }

    fn is_inactive(&self, now: DateTime<Utc>) -> bool {
        let last_active = self.last_seen_at.unwrap_or(self.paired_at);
        now.signed_duration_since(last_active) > Duration::days(DEVICE_INACTIVITY_LIMIT_DAYS)
    }

    fn summary(&self, connected: bool) -> DeviceSummary {
        DeviceSummary {
            id: self.id,
            name: self.name.clone(),
            platform: self.platform.clone(),
            capabilities: self.capabilities.clone(),
            connected,
            last_seen_at: self.last_seen_at,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingClaimSummary {
    pub id: PairingClaimId,
    pub device_name: String,
    pub platform: DevicePlatform,
    pub public_key_fingerprint: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub struct AuthenticatedSession {
    pub response: Authenticated,
    pub cookie_token: String,
}

#[derive(Clone, Debug)]
pub struct SessionAuthorization {
    pub session_id: AuthSessionId,
    pub device_id: DeviceId,
    pub device_name: String,
    pub capabilities: Vec<Capability>,
}

#[derive(Clone, Debug)]
pub struct PairingManager {
    state: Arc<Mutex<PairingState>>,
}

#[derive(Debug)]
struct PairingState {
    registry: DeviceRegistry,
    invitations: HashMap<PairingInvitationId, Invitation>,
    claims: HashMap<PairingClaimId, Claim>,
    challenges: HashMap<ChallengeId, Challenge>,
    sessions: HashMap<[u8; 32], Session>,
    security_attempts: VecDeque<DateTime<Utc>>,
}

#[derive(Debug)]
struct Invitation {
    secret_hash: [u8; 32],
    expires_at: DateTime<Utc>,
}

#[derive(Debug)]
struct Claim {
    claim_secret_hash: [u8; 32],
    device_name: String,
    platform: DevicePlatform,
    public_key_p256_raw: String,
    public_key_fingerprint: String,
    expires_at: DateTime<Utc>,
    resolution: ClaimResolution,
}

#[derive(Clone, Debug)]
enum ClaimResolution {
    Pending,
    Approved {
        device_id: DeviceId,
        capabilities: Vec<Capability>,
    },
    Rejected,
}

#[derive(Debug)]
struct Challenge {
    device_id: DeviceId,
    bytes: [u8; 32],
    expires_at: DateTime<Utc>,
}

#[derive(Debug)]
struct Session {
    id: AuthSessionId,
    device_id: DeviceId,
    expires_at: DateTime<Utc>,
    connected: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PairingErrorKind {
    InvalidRequest,
    Unauthorized,
    NotFound,
    Expired,
    AlreadyUsed,
    Capacity,
    RateLimited,
    Revoked,
    Internal,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum PairingError {
    #[error("invalid pairing or authentication request")]
    InvalidRequest,
    #[error("pairing or authentication secret is invalid")]
    Unauthorized,
    #[error("pairing invitation, claim, challenge, or device was not found")]
    NotFound,
    #[error("pairing invitation, claim, challenge, session, or device authorization expired")]
    Expired,
    #[error("pairing invitation was already used")]
    AlreadyUsed,
    #[error("remote-control pairing capacity has been reached")]
    Capacity,
    #[error("too many pairing or authentication attempts")]
    RateLimited,
    #[error("this device authorization was revoked")]
    Revoked,
    #[error("device registry version {0} is unsupported")]
    UnsupportedRegistryVersion(u16),
    #[error("device registry is invalid")]
    InvalidRegistry,
    #[error("pairing state is unavailable")]
    StateUnavailable,
}

impl PairingError {
    pub fn kind(&self) -> PairingErrorKind {
        match self {
            Self::InvalidRequest | Self::InvalidRegistry | Self::UnsupportedRegistryVersion(_) => {
                PairingErrorKind::InvalidRequest
            }
            Self::Unauthorized => PairingErrorKind::Unauthorized,
            Self::NotFound => PairingErrorKind::NotFound,
            Self::Expired => PairingErrorKind::Expired,
            Self::AlreadyUsed => PairingErrorKind::AlreadyUsed,
            Self::Capacity => PairingErrorKind::Capacity,
            Self::RateLimited => PairingErrorKind::RateLimited,
            Self::Revoked => PairingErrorKind::Revoked,
            Self::StateUnavailable => PairingErrorKind::Internal,
        }
    }
}

impl PairingManager {
    pub fn new(registry: DeviceRegistry) -> Result<Self, PairingError> {
        Ok(Self {
            state: Arc::new(Mutex::new(PairingState {
                registry: registry.validate()?,
                invitations: HashMap::new(),
                claims: HashMap::new(),
                challenges: HashMap::new(),
                sessions: HashMap::new(),
                security_attempts: VecDeque::new(),
            })),
        })
    }

    pub fn registry_snapshot(&self) -> Result<DeviceRegistry, PairingError> {
        Ok(self.lock()?.registry.clone())
    }

    pub fn route_path(&self) -> Result<String, PairingError> {
        Ok(self.lock()?.registry.route_path())
    }

    pub fn create_invitation(
        &self,
        base_url: &str,
        now: DateTime<Utc>,
    ) -> Result<PairingInvitation, PairingError> {
        let mut state = self.lock()?;
        prune(&mut state, now);
        state.invitations.clear();

        let id = PairingInvitationId::new();
        let secret = random_secret();
        let expires_at = now + Duration::seconds(PAIRING_INVITATION_TTL_SECS as i64);
        state.invitations.insert(
            id,
            Invitation {
                secret_hash: secret_hash(&secret),
                expires_at,
            },
        );
        let route_path = state.registry.route_path();
        Ok(PairingInvitation {
            id,
            pairing_url: format!(
                "{}/{}/pair#{}:{}",
                base_url.trim_end_matches('/'),
                route_path.trim_start_matches('/'),
                id,
                secret
            ),
            expires_at,
        })
    }

    pub fn claim(
        &self,
        request: PairingClaimRequest,
        now: DateTime<Utc>,
    ) -> Result<PairingClaimReceipt, PairingError> {
        request
            .validate()
            .map_err(|_| PairingError::InvalidRequest)?;
        if request.device_name.chars().any(char::is_control) {
            return Err(PairingError::InvalidRequest);
        }

        let mut state = self.lock()?;
        check_rate_limit(&mut state, now)?;
        prune(&mut state, now);
        if state.claims.len() >= MAX_PENDING_CLAIMS {
            return Err(PairingError::Capacity);
        }
        let Some(invitation) = state.invitations.remove(&request.invitation_id) else {
            return Err(PairingError::NotFound);
        };
        if invitation.expires_at <= now {
            return Err(PairingError::Expired);
        }
        if !secrets_match(&invitation.secret_hash, &request.secret) {
            return Err(PairingError::Unauthorized);
        }
        if state
            .registry
            .devices
            .iter()
            .filter(|device| device.revoked_at.is_none())
            .count()
            >= MAX_PAIRED_DEVICES
        {
            return Err(PairingError::Capacity);
        }

        let public_key = BASE64_STANDARD
            .decode(&request.public_key_p256_raw)
            .map_err(|_| PairingError::InvalidRequest)?;
        let public_key_fingerprint = hex::encode(Sha256::digest(&public_key));
        let claim_id = PairingClaimId::new();
        let claim_secret = random_secret();
        let expires_at = invitation.expires_at;
        state.claims.insert(
            claim_id,
            Claim {
                claim_secret_hash: secret_hash(&claim_secret),
                device_name: request.device_name.clone(),
                platform: request.platform,
                public_key_p256_raw: request.public_key_p256_raw,
                public_key_fingerprint: public_key_fingerprint.clone(),
                expires_at,
                resolution: ClaimResolution::Pending,
            },
        );

        Ok(PairingClaimReceipt {
            claim_id,
            claim_secret,
            device_name: request.device_name,
            public_key_fingerprint,
            expires_at,
        })
    }

    pub fn cancel_invitation(
        &self,
        invitation_id: PairingInvitationId,
    ) -> Result<(), PairingError> {
        self.lock()?.invitations.remove(&invitation_id);
        Ok(())
    }

    pub fn pending_claims(
        &self,
        now: DateTime<Utc>,
    ) -> Result<Vec<PendingClaimSummary>, PairingError> {
        let mut state = self.lock()?;
        prune(&mut state, now);
        Ok(state
            .claims
            .iter()
            .filter_map(|(id, claim)| {
                matches!(claim.resolution, ClaimResolution::Pending).then(|| PendingClaimSummary {
                    id: *id,
                    device_name: claim.device_name.clone(),
                    platform: claim.platform.clone(),
                    public_key_fingerprint: claim.public_key_fingerprint.clone(),
                    expires_at: claim.expires_at,
                })
            })
            .collect())
    }

    pub fn approve(
        &self,
        claim_id: PairingClaimId,
        capabilities: Vec<Capability>,
        now: DateTime<Utc>,
    ) -> Result<PairedDeviceRecord, PairingError> {
        let mut state = self.lock()?;
        prune(&mut state, now);
        if capabilities.is_empty() {
            return Err(PairingError::InvalidRequest);
        }
        let (device_name, platform, public_key_p256_raw) = {
            let claim = state.claims.get(&claim_id).ok_or(PairingError::NotFound)?;
            if !matches!(claim.resolution, ClaimResolution::Pending) {
                return Err(PairingError::AlreadyUsed);
            }
            (
                claim.device_name.clone(),
                claim.platform.clone(),
                claim.public_key_p256_raw.clone(),
            )
        };

        // Revoked records are retained long enough to reject stale keys explicitly, but they must
        // not make the bounded registry grow forever as phones are replaced over time.
        while state.registry.devices.len() >= MAX_PAIRED_DEVICES {
            let Some(index) = state
                .registry
                .devices
                .iter()
                .position(|device| device.revoked_at.is_some())
            else {
                return Err(PairingError::Capacity);
            };
            state.registry.devices.remove(index);
        }

        let record = PairedDeviceRecord {
            id: DeviceId::new(),
            name: device_name,
            platform,
            capabilities: capabilities.clone(),
            public_key_p256_raw,
            paired_at: now,
            last_seen_at: None,
            revoked_at: None,
        };
        state
            .claims
            .get_mut(&claim_id)
            .expect("claim was checked above")
            .resolution = ClaimResolution::Approved {
            device_id: record.id,
            capabilities,
        };
        state.registry.devices.push(record.clone());
        Ok(record)
    }

    pub fn reject(&self, claim_id: PairingClaimId, now: DateTime<Utc>) -> Result<(), PairingError> {
        let mut state = self.lock()?;
        prune(&mut state, now);
        let claim = state
            .claims
            .get_mut(&claim_id)
            .ok_or(PairingError::NotFound)?;
        if !matches!(claim.resolution, ClaimResolution::Pending) {
            return Err(PairingError::AlreadyUsed);
        }
        claim.resolution = ClaimResolution::Rejected;
        Ok(())
    }

    pub fn pairing_status(
        &self,
        request: PairingStatusRequest,
        now: DateTime<Utc>,
    ) -> Result<PairingStatus, PairingError> {
        request
            .validate()
            .map_err(|_| PairingError::InvalidRequest)?;
        let mut state = self.lock()?;
        check_rate_limit(&mut state, now)?;
        prune(&mut state, now);
        let claim = state
            .claims
            .get(&request.claim_id)
            .ok_or(PairingError::NotFound)?;
        if !secrets_match(&claim.claim_secret_hash, &request.claim_secret) {
            return Err(PairingError::Unauthorized);
        }
        Ok(match &claim.resolution {
            ClaimResolution::Pending => PairingStatus::Pending,
            ClaimResolution::Approved {
                device_id,
                capabilities,
            } => PairingStatus::Approved {
                device_id: *device_id,
                capabilities: capabilities.clone(),
            },
            ClaimResolution::Rejected => PairingStatus::Rejected,
        })
    }

    pub fn create_challenge(
        &self,
        request: AuthChallengeRequest,
        now: DateTime<Utc>,
    ) -> Result<AuthChallenge, PairingError> {
        let mut state = self.lock()?;
        check_rate_limit(&mut state, now)?;
        prune(&mut state, now);
        let device = state
            .registry
            .devices
            .iter()
            .find(|device| device.id == request.device_id)
            .ok_or(PairingError::NotFound)?;
        ensure_device_active(device, now)?;
        if state.challenges.len() >= MAX_CHALLENGES {
            return Err(PairingError::Capacity);
        }

        let id = ChallengeId::new();
        let mut bytes = [0_u8; 32];
        OsRng.fill_bytes(&mut bytes);
        let expires_at = now + Duration::seconds(AUTH_CHALLENGE_TTL_SECS as i64);
        state.challenges.insert(
            id,
            Challenge {
                device_id: request.device_id,
                bytes,
                expires_at,
            },
        );
        Ok(AuthChallenge {
            id,
            device_id: request.device_id,
            challenge: BASE64_STANDARD.encode(bytes),
            expires_at,
        })
    }

    pub fn authenticate(
        &self,
        request: Authenticate,
        now: DateTime<Utc>,
    ) -> Result<AuthenticatedSession, PairingError> {
        request
            .validate()
            .map_err(|_| PairingError::InvalidRequest)?;
        let mut state = self.lock()?;
        check_rate_limit(&mut state, now)?;
        prune(&mut state, now);
        let challenge = state
            .challenges
            .remove(&request.challenge_id)
            .ok_or(PairingError::NotFound)?;
        if challenge.device_id != request.device_id {
            return Err(PairingError::Unauthorized);
        }
        if challenge.expires_at <= now {
            return Err(PairingError::Expired);
        }

        let device_index = state
            .registry
            .devices
            .iter()
            .position(|device| device.id == request.device_id)
            .ok_or(PairingError::NotFound)?;
        ensure_device_active(&state.registry.devices[device_index], now)?;
        let public_key = BASE64_STANDARD
            .decode(&state.registry.devices[device_index].public_key_p256_raw)
            .map_err(|_| PairingError::InvalidRegistry)?;
        let signature_bytes = BASE64_STANDARD
            .decode(&request.signature)
            .map_err(|_| PairingError::InvalidRequest)?;
        signature::UnparsedPublicKey::new(&signature::ECDSA_P256_SHA256_FIXED, public_key)
            .verify(&challenge.bytes, &signature_bytes)
            .map_err(|_| PairingError::Unauthorized)?;

        let active_sessions = state
            .sessions
            .values()
            .filter(|session| {
                session.device_id == request.device_id
                    && session.connected
                    && session.expires_at > now
            })
            .count();
        if active_sessions >= MAX_CONNECTIONS_PER_DEVICE {
            return Err(PairingError::Capacity);
        }

        // An HTTP authentication can succeed even when the following WebSocket handshake is
        // interrupted. Replace that device's unclaimed cookie instead of leaking connection slots
        // until the 15-minute session expiry.
        state
            .sessions
            .retain(|_, session| session.device_id != request.device_id || session.connected);

        state.registry.devices[device_index].last_seen_at = Some(now);
        let device = state.registry.devices[device_index].summary(false);
        let session_id = AuthSessionId::new();
        let cookie_token = random_secret();
        let expires_at = now + Duration::seconds(AUTH_SESSION_TTL_SECS as i64);
        state.sessions.insert(
            secret_hash(&cookie_token),
            Session {
                id: session_id,
                device_id: request.device_id,
                expires_at,
                connected: false,
            },
        );

        Ok(AuthenticatedSession {
            response: Authenticated {
                session_id,
                device,
                expires_at,
                // Every WebSocket receives an authoritative snapshot immediately. A future
                // bounded event-log implementation can populate this only when it truly replays.
                replayed_from_sequence: None,
            },
            cookie_token,
        })
    }

    pub fn authorize_session(
        &self,
        cookie_token: &str,
        now: DateTime<Utc>,
    ) -> Result<SessionAuthorization, PairingError> {
        let mut state = self.lock()?;
        prune(&mut state, now);
        let token_hash = secret_hash(cookie_token);
        let session = state
            .sessions
            .get(&token_hash)
            .ok_or(PairingError::Unauthorized)?;
        let device = state
            .registry
            .devices
            .iter()
            .find(|device| device.id == session.device_id)
            .ok_or(PairingError::NotFound)?;
        ensure_device_active(device, now)?;
        Ok(SessionAuthorization {
            session_id: session.id,
            device_id: device.id,
            device_name: device.name.clone(),
            capabilities: device.capabilities.clone(),
        })
    }

    /// Claims an authenticated HTTP cookie for a live WebSocket connection.
    pub fn connect_session(
        &self,
        cookie_token: &str,
        now: DateTime<Utc>,
    ) -> Result<SessionAuthorization, PairingError> {
        let mut state = self.lock()?;
        prune(&mut state, now);
        let token_hash = secret_hash(cookie_token);
        let (session_id, device_id, already_connected) = state
            .sessions
            .get(&token_hash)
            .map(|session| (session.id, session.device_id, session.connected))
            .ok_or(PairingError::Unauthorized)?;
        let device = state
            .registry
            .devices
            .iter()
            .find(|device| device.id == device_id)
            .ok_or(PairingError::NotFound)?;
        ensure_device_active(device, now)?;
        let authorization = SessionAuthorization {
            session_id,
            device_id: device.id,
            device_name: device.name.clone(),
            capabilities: device.capabilities.clone(),
        };
        // A short-lived cookie authorizes one WebSocket, not an arbitrary number of tabs that
        // happen to share browser storage. Reconnects authenticate again and receive a new cookie.
        if already_connected {
            return Err(PairingError::AlreadyUsed);
        }
        let active_sessions = state
            .sessions
            .values()
            .filter(|session| {
                session.device_id == device_id && session.connected && session.expires_at > now
            })
            .count();
        if active_sessions >= MAX_CONNECTIONS_PER_DEVICE {
            return Err(PairingError::Capacity);
        }
        if let Some(session) = state.sessions.get_mut(&token_hash) {
            session.connected = true;
        }
        Ok(authorization)
    }

    pub fn end_session(&self, session_id: AuthSessionId) -> Result<(), PairingError> {
        let mut state = self.lock()?;
        state.sessions.retain(|_, session| session.id != session_id);
        Ok(())
    }

    pub fn paired_devices(&self, now: DateTime<Utc>) -> Result<Vec<DeviceSummary>, PairingError> {
        let mut state = self.lock()?;
        prune(&mut state, now);
        let connected_ids: Vec<_> = state
            .sessions
            .values()
            .filter(|session| session.connected)
            .map(|session| session.device_id)
            .collect();
        Ok(state
            .registry
            .devices
            .iter()
            .filter(|device| device.revoked_at.is_none())
            .map(|device| device.summary(connected_ids.contains(&device.id)))
            .collect())
    }

    pub fn revoke_device(
        &self,
        device_id: DeviceId,
        now: DateTime<Utc>,
    ) -> Result<(), PairingError> {
        let mut state = self.lock()?;
        let device = state
            .registry
            .devices
            .iter_mut()
            .find(|device| device.id == device_id)
            .ok_or(PairingError::NotFound)?;
        device.revoked_at = Some(now);
        state
            .sessions
            .retain(|_, session| session.device_id != device_id);
        state
            .challenges
            .retain(|_, challenge| challenge.device_id != device_id);
        Ok(())
    }

    pub fn revoke_all_devices(&self, now: DateTime<Utc>) -> Result<(), PairingError> {
        let mut state = self.lock()?;
        for device in &mut state.registry.devices {
            if device.revoked_at.is_none() {
                device.revoked_at = Some(now);
            }
        }
        state.invitations.clear();
        state.claims.clear();
        state.challenges.clear();
        state.sessions.clear();
        state.security_attempts.clear();
        Ok(())
    }

    pub fn invalidate_ephemeral_state(&self) -> Result<(), PairingError> {
        let mut state = self.lock()?;
        state.invitations.clear();
        state.claims.clear();
        state.challenges.clear();
        state.sessions.clear();
        state.security_attempts.clear();
        Ok(())
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, PairingState>, PairingError> {
        self.state
            .lock()
            .map_err(|_| PairingError::StateUnavailable)
    }
}

fn ensure_device_active(
    device: &PairedDeviceRecord,
    now: DateTime<Utc>,
) -> Result<(), PairingError> {
    if device.revoked_at.is_some() {
        return Err(PairingError::Revoked);
    }
    if device.is_inactive(now) {
        return Err(PairingError::Expired);
    }
    Ok(())
}

fn check_rate_limit(state: &mut PairingState, now: DateTime<Utc>) -> Result<(), PairingError> {
    let cutoff = now - Duration::minutes(1);
    while state
        .security_attempts
        .front()
        .is_some_and(|attempt| *attempt <= cutoff)
    {
        state.security_attempts.pop_front();
    }
    if state.security_attempts.len() >= MAX_SECURITY_ATTEMPTS_PER_MINUTE {
        return Err(PairingError::RateLimited);
    }
    state.security_attempts.push_back(now);
    Ok(())
}

fn prune(state: &mut PairingState, now: DateTime<Utc>) {
    state
        .invitations
        .retain(|_, invitation| invitation.expires_at > now);
    state.claims.retain(|_, claim| claim.expires_at > now);
    state
        .challenges
        .retain(|_, challenge| challenge.expires_at > now);
    state.sessions.retain(|_, session| session.expires_at > now);
}

fn random_secret() -> String {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn random_route_id() -> String {
    let mut bytes = [0_u8; 12];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

fn secret_hash(secret: &str) -> [u8; 32] {
    Sha256::digest(secret.as_bytes()).into()
}

fn secrets_match(expected_hash: &[u8; 32], candidate: &str) -> bool {
    bool::from(expected_hash.ct_eq(&secret_hash(candidate)))
}

#[cfg(test)]
mod tests {
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
        let pkcs8 = signature::EcdsaKeyPair::generate_pkcs8(
            &signature::ECDSA_P256_SHA256_FIXED_SIGNING,
            rng,
        )
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
}
