use serde::{Deserialize, Serialize};

use super::domain::IncomingMessage;

pub(crate) const BRIDGE_PROTOCOL_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize)]
pub(crate) struct BridgeRequest {
    pub(crate) version: u32,
    pub(crate) id: String,
    #[serde(flatten)]
    pub(crate) command: BridgeCommand,
}

impl BridgeRequest {
    pub(crate) fn new(id: impl Into<String>, command: BridgeCommand) -> Self {
        Self {
            version: BRIDGE_PROTOCOL_VERSION,
            id: id.into(),
            command,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub(crate) enum BridgeCommand {
    Health,
    Configure {
        recipient: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        chat_id: Option<i64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        chat_guid: Option<String>,
    },
    Send {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        route_id: Option<String>,
    },
    StartWatch {
        chat_id: i64,
        after_row_id: i64,
    },
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct BridgeResponse {
    pub(crate) version: u32,
    pub(crate) id: String,
    pub(crate) ok: bool,
    #[serde(default)]
    pub(crate) result: Option<BridgeResult>,
    #[serde(default)]
    pub(crate) error: Option<BridgeError>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum BridgeResult {
    Health {
        database_readable: bool,
        automation_authorized: bool,
    },
    Configured {
        #[serde(default)]
        chat_id: Option<i64>,
        #[serde(default)]
        chat_guid: Option<String>,
    },
    Sent {
        guid: String,
        row_id: i64,
        chat_id: i64,
        #[serde(default)]
        chat_guid: Option<String>,
    },
    Watching,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct BridgeError {
    pub(crate) code: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub(crate) enum BridgeEvent {
    Incoming {
        version: u32,
        message: IncomingMessage,
    },
    PermissionRequired {
        version: u32,
        permission: BridgePermission,
    },
    DeliveryFailed {
        version: u32,
        code: String,
    },
    WatchFailed {
        version: u32,
        code: String,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BridgePermission {
    Automation,
    FullDiskAccess,
    MessagesSignIn,
}
