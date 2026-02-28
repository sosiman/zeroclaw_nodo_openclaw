use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The protocol version of the OpenClaw WS gateway we support.
pub const PROTOCOL_VERSION: u32 = 3;

/// Represents a client trying to connect to the gateway.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayClient {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub version: String,
    pub platform: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_family: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_identifier: Option<String>,
    pub mode: String, // e.g., "desktop", "cli", "headless"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance_id: Option<String>,
}

/// Information needed when connecting via a paired ED25519 device key
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectDevice {
    pub id: String,
    pub public_key: String,
    pub signature: String,
    pub signed_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nonce: Option<String>,
}

/// The parameters sent in the initial connect frame.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectParams {
    pub min_protocol: u32,
    pub max_protocol: u32,
    pub client: GatewayClient,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caps: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commands: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<ConnectDevice>,
}

/// The response to a successful connection
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HelloOk {
    pub protocol: u32,
    pub server: ServerInfo,
    pub policy: GatewayPolicy,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerInfo {
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    pub conn_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayPolicy {
    pub max_payload: u32,
    pub max_buffered_bytes: u32,
    pub tick_interval_ms: u32,
}

/// Format of a Gateway error response payload
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorShape {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retryable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
}

/// Unified generic Gateway Frame structure based on discriminator "type"
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum GatewayFrame {
    #[serde(rename = "req")]
    Request(RequestFrame),
    #[serde(rename = "res")]
    Response(ResponseFrame),
    #[serde(rename = "event")]
    Event(EventFrame),
    // hello-ok from server
    #[serde(rename = "hello-ok")]
    HelloOk(HelloOk),
}

/// A request frame sent to invoke a method
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestFrame {
    pub id: String,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

/// A response frame from the server or from us
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResponseFrame {
    pub id: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorShape>,
}

/// Event streams coming from the server
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventFrame {
    pub event: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
}

/// Nodes API: The invoke execution struct we will receive.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeInvokeRequestEvent {
    pub id: String,
    pub node_id: String,
    pub command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params_json: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
}

/// Generic Helper trait to create quick error frames
impl ResponseFrame {
    pub fn new_err(id: String, code: &str, message: &str) -> Self {
        Self {
            id,
            ok: false,
            payload: None,
            error: Some(ErrorShape {
                code: code.to_string(),
                message: message.to_string(),
                details: None,
                retryable: None,
                retry_after_ms: None,
            }),
        }
    }

    pub fn new_ok(id: String, payload: Value) -> Self {
        Self {
            id,
            ok: true,
            payload: Some(payload),
            error: None,
        }
    }
}
