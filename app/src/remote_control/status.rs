use clinch_companion_protocol::{DeviceSummary, PairingInvitation};

use super::pairing::PendingClaimSummary;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RemoteControlStatus {
    Disabled,
    Starting,
    TailscaleNotInstalled,
    TailscaleStopped,
    TailscaleSignInRequired {
        action_url: Option<String>,
    },
    TailscaleConsentRequired {
        action_url: Option<String>,
    },
    Ready {
        remote_url: String,
        loopback_port: u16,
    },
    Error {
        message: String,
        retryable: bool,
    },
}

impl RemoteControlStatus {
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready { .. })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteControlViewState {
    pub enabled: bool,
    pub status: RemoteControlStatus,
    pub active_invitation: Option<PairingInvitation>,
    pub pending_claims: Vec<PendingClaimSummary>,
    pub paired_devices: Vec<DeviceSummary>,
}

impl Default for RemoteControlViewState {
    fn default() -> Self {
        Self {
            enabled: false,
            status: RemoteControlStatus::Disabled,
            active_invitation: None,
            pending_claims: Vec::new(),
            paired_devices: Vec::new(),
        }
    }
}
