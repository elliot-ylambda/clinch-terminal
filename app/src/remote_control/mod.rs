mod gateway;
mod pairing;
mod status;
mod tailscale;
mod workspace_adapter;

use std::time::Duration as StdDuration;

use chrono::Utc;
use clinch_companion_protocol::{Capability, DeviceId, PairingClaimId, PairingInvitation};
use settings::Setting as _;
use warp_core::channel::ChannelState;
use warpui::{AppContext, Entity, ModelContext, SingletonEntity};
use warpui_extras::secure_storage;

pub use status::{RemoteControlStatus, RemoteControlViewState};

use self::gateway::{GatewayEvent, GatewayHandle};
use self::pairing::{DeviceRegistry, PairingManager};
use self::tailscale::{TailscaleClient, TailscaleError, TailscaleSetupOutcome};
use self::workspace_adapter::WorkspaceAdapter;
use crate::settings::{RemoteControlMode, RemoteControlSettings};

const DEVICE_REGISTRY_STORAGE_KEY: &str = "ClinchRemoteControlDeviceRegistryV1";

pub fn register(ctx: &mut AppContext) {
    let registry = if ChannelState::has_backend() {
        // The inherited account-backed app still constructs the Clinch settings page, so its
        // model handle must exist. Give it an inert in-memory authority without reading or
        // rewriting Clinch's local device registry.
        DeviceRegistry::default()
    } else {
        load_registry(ctx)
    };
    let pairing = PairingManager::new(registry).unwrap_or_else(|error| {
        log::error!("Remote Control device registry was rejected: {error}");
        PairingManager::new(DeviceRegistry::default()).expect("default device registry is valid")
    });
    if !ChannelState::has_backend() {
        persist_registry(&pairing, ctx);
    }

    let workspace_pairing = pairing.clone();
    ctx.add_singleton_model(move |ctx| WorkspaceAdapter::new(workspace_pairing, ctx));
    ctx.add_singleton_model(move |ctx| RemoteControlService::new(pairing, ctx));
}

pub struct RemoteControlService {
    pairing: PairingManager,
    view_state: RemoteControlViewState,
    runtime: Option<tokio::runtime::Runtime>,
    gateway: Option<GatewayHandle>,
    base_origin: Option<String>,
    generation: u64,
    cleanup_in_progress: bool,
}

impl Entity for RemoteControlService {
    type Event = ();
}

impl SingletonEntity for RemoteControlService {}

impl RemoteControlService {
    fn new(pairing: PairingManager, ctx: &mut ModelContext<Self>) -> Self {
        let mut service = Self {
            pairing,
            view_state: RemoteControlViewState::default(),
            runtime: None,
            gateway: None,
            base_origin: None,
            generation: 0,
            cleanup_in_progress: false,
        };
        ctx.subscribe_to_model(&RemoteControlSettings::handle(ctx), |service, _, _, ctx| {
            service.refresh_for_settings(ctx);
        });
        service.refresh_for_settings(ctx);
        service
    }

    pub fn view_state(&self) -> &RemoteControlViewState {
        &self.view_state
    }

    pub fn set_enabled(&mut self, enabled: bool, ctx: &mut ModelContext<Self>) {
        if ChannelState::has_backend() {
            return;
        }
        RemoteControlSettings::handle(ctx).update(ctx, |settings, ctx| {
            let mode = if enabled {
                RemoteControlMode::Enabled
            } else {
                RemoteControlMode::Disabled
            };
            if let Err(error) = settings.mode.set_value(mode, ctx) {
                log::error!("could not persist Remote Control enablement: {error}");
            }
        });
    }

    pub fn retry(&mut self, ctx: &mut ModelContext<Self>) {
        if ChannelState::has_backend()
            || !RemoteControlSettings::as_ref(ctx).is_enabled()
            || self.cleanup_in_progress
        {
            return;
        }
        self.stop_runtime(false, ctx);
        self.start(ctx);
    }

    pub fn create_pairing_invitation(
        &mut self,
        ctx: &mut ModelContext<Self>,
    ) -> Result<PairingInvitation, String> {
        let base_origin = self
            .base_origin
            .as_deref()
            .ok_or_else(|| "Remote Control is not ready yet.".to_owned())?;
        let invitation = self
            .pairing
            .create_invitation(base_origin, Utc::now())
            .map_err(|error| error.to_string())?;
        self.view_state.active_invitation = Some(invitation.clone());
        ctx.notify();
        Ok(invitation)
    }

    pub fn cancel_pairing_invitation(&mut self, ctx: &mut ModelContext<Self>) {
        if let Some(invitation) = self.view_state.active_invitation.take() {
            let _ = self.pairing.cancel_invitation(invitation.id);
        }
        ctx.notify();
    }

    pub fn approve_pairing(
        &mut self,
        claim_id: PairingClaimId,
        ctx: &mut ModelContext<Self>,
    ) -> Result<(), String> {
        self.pairing
            .approve(
                claim_id,
                vec![
                    Capability::View,
                    Capability::Control,
                    Capability::CreateSession,
                    Capability::Upload,
                ],
                Utc::now(),
            )
            .map_err(|error| error.to_string())?;
        persist_registry(&self.pairing, ctx);
        self.refresh_pairing_state();
        ctx.notify();
        Ok(())
    }

    pub fn reject_pairing(
        &mut self,
        claim_id: PairingClaimId,
        ctx: &mut ModelContext<Self>,
    ) -> Result<(), String> {
        self.pairing
            .reject(claim_id, Utc::now())
            .map_err(|error| error.to_string())?;
        self.refresh_pairing_state();
        ctx.notify();
        Ok(())
    }

    pub fn revoke_device(
        &mut self,
        device_id: DeviceId,
        ctx: &mut ModelContext<Self>,
    ) -> Result<(), String> {
        self.pairing
            .revoke_device(device_id, Utc::now())
            .map_err(|error| error.to_string())?;
        persist_registry(&self.pairing, ctx);
        self.refresh_pairing_state();
        ctx.notify();
        Ok(())
    }

    pub fn revoke_all_devices(&mut self, ctx: &mut ModelContext<Self>) -> Result<(), String> {
        self.pairing
            .revoke_all_devices(Utc::now())
            .map_err(|error| error.to_string())?;
        persist_registry(&self.pairing, ctx);
        self.view_state.active_invitation = None;
        self.refresh_pairing_state();
        ctx.notify();
        Ok(())
    }

    fn refresh_for_settings(&mut self, ctx: &mut ModelContext<Self>) {
        if ChannelState::has_backend() {
            // Remote Control is a backend-free Clinch feature. Inherited Warp channels keep an
            // inert model only because their shared settings view constructs every page eagerly.
            self.view_state.enabled = false;
            self.view_state.status = RemoteControlStatus::Disabled;
            self.view_state.active_invitation = None;
            self.view_state.pending_claims.clear();
            self.view_state.paired_devices.clear();
            ctx.notify();
            return;
        }
        let enabled = RemoteControlSettings::as_ref(ctx).is_enabled();
        self.view_state.enabled = enabled;
        if enabled {
            if self.cleanup_in_progress {
                self.view_state.status = RemoteControlStatus::Starting;
            } else if self.runtime.is_none() {
                self.start(ctx);
            }
        } else {
            self.stop_runtime(true, ctx);
        }
        ctx.notify();
    }

    fn start(&mut self, ctx: &mut ModelContext<Self>) {
        if ChannelState::has_backend() {
            self.view_state.status = RemoteControlStatus::Disabled;
            return;
        }
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.view_state.status = RemoteControlStatus::Starting;
        self.view_state.active_invitation = None;
        self.base_origin = None;

        let runtime = match tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                self.view_state.status = RemoteControlStatus::Error {
                    message: format!("Could not start the private companion: {error}"),
                    retryable: true,
                };
                return;
            }
        };
        let route_path = match self.pairing.route_path() {
            Ok(route_path) => route_path,
            Err(error) => {
                self.view_state.status = RemoteControlStatus::Error {
                    message: error.to_string(),
                    retryable: false,
                };
                runtime.shutdown_background();
                return;
            }
        };
        let (event_tx, event_rx) = async_channel::bounded(64);
        let workspace_spawner = WorkspaceAdapter::handle(ctx).update(ctx, |_, ctx| ctx.spawner());
        let gateway = match GatewayHandle::start(
            &runtime,
            route_path.clone(),
            self.pairing.clone(),
            workspace_spawner,
            event_tx,
            gateway::locate_assets(),
        ) {
            Ok(gateway) => gateway,
            Err(error) => {
                self.view_state.status = RemoteControlStatus::Error {
                    message: error.to_string(),
                    retryable: true,
                };
                runtime.shutdown_background();
                return;
            }
        };
        let port = gateway.port;
        let spawner = ctx.spawner();
        runtime.spawn(async move {
            while let Ok(event) = event_rx.recv().await {
                if spawner
                    .spawn(move |service, ctx| service.handle_gateway_event(event, ctx))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });

        let client = match TailscaleClient::discover() {
            Ok(client) => client,
            Err(TailscaleError::NotInstalled) => {
                self.view_state.status = RemoteControlStatus::TailscaleNotInstalled;
                self.gateway = Some(gateway);
                self.runtime = Some(runtime);
                return;
            }
            Err(error) => {
                self.view_state.status = RemoteControlStatus::Error {
                    message: error.to_string(),
                    retryable: true,
                };
                self.gateway = Some(gateway);
                self.runtime = Some(runtime);
                return;
            }
        };
        let setup_spawner = ctx.spawner();
        runtime.spawn(async move {
            let result = client.configure_private_route(&route_path, port).await;
            let _ = setup_spawner
                .spawn(move |service, ctx| {
                    service.finish_tailscale_setup(generation, port, result, ctx)
                })
                .await;
        });
        self.gateway = Some(gateway);
        self.runtime = Some(runtime);
    }

    fn finish_tailscale_setup(
        &mut self,
        generation: u64,
        port: u16,
        result: Result<TailscaleSetupOutcome, TailscaleError>,
        ctx: &mut ModelContext<Self>,
    ) {
        if generation != self.generation || !self.view_state.enabled {
            return;
        }
        self.view_state.status = match result {
            Ok(TailscaleSetupOutcome::Ready(ready)) => {
                if let Some(gateway) = &self.gateway {
                    if let Err(error) = gateway.security.set_public_origin(&ready.base_url) {
                        RemoteControlStatus::Error {
                            message: error.to_string(),
                            retryable: true,
                        }
                    } else {
                        self.base_origin = Some(ready.base_url.clone());
                        RemoteControlStatus::Ready {
                            remote_url: format!(
                                "{}/{}",
                                ready.base_url.trim_end_matches('/'),
                                ready.route_path.trim_start_matches('/')
                            ),
                            loopback_port: port,
                        }
                    }
                } else {
                    RemoteControlStatus::Error {
                        message: "The private companion stopped during setup.".to_owned(),
                        retryable: true,
                    }
                }
            }
            Ok(TailscaleSetupOutcome::Stopped) => RemoteControlStatus::TailscaleStopped,
            Ok(TailscaleSetupOutcome::SignInRequired { action_url }) => {
                RemoteControlStatus::TailscaleSignInRequired { action_url }
            }
            Ok(TailscaleSetupOutcome::ConsentRequired { action_url }) => {
                RemoteControlStatus::TailscaleConsentRequired { action_url }
            }
            Err(TailscaleError::NotInstalled) => RemoteControlStatus::TailscaleNotInstalled,
            Err(error) => RemoteControlStatus::Error {
                message: error.to_string(),
                retryable: true,
            },
        };
        self.refresh_pairing_state();
        ctx.notify();
    }

    fn handle_gateway_event(&mut self, event: GatewayEvent, ctx: &mut ModelContext<Self>) {
        match event {
            GatewayEvent::DeviceRegistryChanged => persist_registry(&self.pairing, ctx),
            GatewayEvent::PendingPairingChanged => self.view_state.active_invitation = None,
            GatewayEvent::ClientConnected | GatewayEvent::ClientDisconnected => {}
        }
        self.refresh_pairing_state();
        ctx.notify();
    }

    fn refresh_pairing_state(&mut self) {
        let now = Utc::now();
        self.view_state.pending_claims = self.pairing.pending_claims(now).unwrap_or_default();
        self.view_state.paired_devices = self.pairing.paired_devices(now).unwrap_or_default();
        if self
            .view_state
            .active_invitation
            .as_ref()
            .is_some_and(|invitation| invitation.expires_at <= now)
        {
            self.view_state.active_invitation = None;
        }
    }

    fn stop_runtime(&mut self, remove_route: bool, ctx: &mut ModelContext<Self>) {
        if self.runtime.is_none() && self.gateway.is_none() {
            self.view_state.status = RemoteControlStatus::Disabled;
            self.base_origin = None;
            self.view_state.active_invitation = None;
            return;
        }
        self.generation = self.generation.wrapping_add(1);
        WorkspaceAdapter::handle(ctx).update(ctx, |adapter, ctx| {
            adapter.all_sessions_disconnected(ctx);
        });
        self.gateway = None;
        let _ = self.pairing.invalidate_ephemeral_state();
        self.base_origin = None;
        self.view_state.active_invitation = None;
        self.view_state.pending_claims.clear();
        self.view_state.status = RemoteControlStatus::Disabled;
        let Some(runtime) = self.runtime.take() else {
            return;
        };
        if !remove_route {
            runtime.shutdown_background();
            return;
        }

        let Ok(client) = TailscaleClient::discover() else {
            runtime.shutdown_background();
            return;
        };
        let Ok(route_path) = self.pairing.route_path() else {
            runtime.shutdown_background();
            return;
        };
        self.cleanup_in_progress = true;
        let spawner = ctx.spawner();
        let _ = std::thread::Builder::new()
            .name("clinch-remote-cleanup".to_owned())
            .spawn(move || {
                let _ = runtime.block_on(client.remove_private_route(&route_path));
                runtime.shutdown_timeout(StdDuration::from_secs(2));
                futures_lite::future::block_on(async move {
                    let _ = spawner
                        .spawn(|service, ctx| service.finish_cleanup(ctx))
                        .await;
                });
            });
    }

    fn finish_cleanup(&mut self, ctx: &mut ModelContext<Self>) {
        self.cleanup_in_progress = false;
        if RemoteControlSettings::as_ref(ctx).is_enabled() {
            self.start(ctx);
        }
        ctx.notify();
    }
}

impl Drop for RemoteControlService {
    fn drop(&mut self) {
        self.gateway = None;
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}

fn load_registry(ctx: &AppContext) -> DeviceRegistry {
    let storage = secure_storage::Model::handle(ctx).as_ref(ctx);
    storage
        .read_value(DEVICE_REGISTRY_STORAGE_KEY)
        .ok()
        .and_then(|json| serde_json::from_str::<DeviceRegistry>(&json).ok())
        .and_then(|registry| registry.validate().ok())
        .unwrap_or_default()
}

fn persist_registry(pairing: &PairingManager, ctx: &AppContext) {
    let Ok(registry) = pairing.registry_snapshot() else {
        return;
    };
    let Ok(json) = serde_json::to_string(&registry) else {
        return;
    };
    if let Err(error) = secure_storage::Model::handle(ctx)
        .as_ref(ctx)
        .write_value_with_owner_only_fallback(DEVICE_REGISTRY_STORAGE_KEY, &json)
    {
        log::error!("could not persist Remote Control device registry: {error}");
    }
}
