use landa_proto::{DeviceCategory, DeviceOs};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerInfo {
    pub peer_id: String,
    pub public_key_hex: String,
    pub sas_code: String,
    pub device_name: String,
    pub alias_name: Option<String>,
    pub display_name: String,
    pub os: DeviceOs,
    pub category: DeviceCategory,
    pub ip: Option<String>,
    pub quic_port: Option<u16>,
    pub is_trusted: bool,
    pub is_online: bool,
    pub last_seen_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FriendRequestInfo {
    pub peer_id: String,
    pub device_name: String,
    pub os: DeviceOs,
    pub category: DeviceCategory,
    pub sas_code: String,
    pub direction: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IncomingTransferOfferInfo {
    pub transfer_id: String,
    pub peer_id: String,
    pub peer_name: String,
    pub file_count: usize,
    pub total_bytes: u64,
    pub file_names: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferProgressInfo {
    pub transfer_id: String,
    pub direction: String,
    pub peer_id: String,
    pub peer_name: String,
    pub current_file: String,
    pub files_done: usize,
    pub file_count: usize,
    pub bytes_transferred: u64,
    pub total_bytes: u64,
    pub speed_bps: u64,
    pub eta_secs: Option<u64>,
    pub status: String,
    pub error: Option<String>,
    #[serde(default)]
    pub saved_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferHistoryRecord {
    pub id: String,
    pub direction: String,
    pub peer_id: String,
    pub peer_name: String,
    pub file_count: usize,
    pub total_bytes: u64,
    pub saved_paths: Vec<String>,
    #[serde(default)]
    pub relative_paths: Vec<String>,
    pub status: String,
    pub error_message: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManualTarget {
    pub ip: String,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "camelCase")]
pub enum CoreEvent {
    PeersChanged,
    FriendRequestsChanged,
    IncomingTransferOffer(IncomingTransferOfferInfo),
    TransferProgress(TransferProgressInfo),
    HistoryChanged,
    #[serde(rename_all = "camelCase")]
    ModuleEvent {
        module_id: String,
        event_name: String,
        data: serde_json::Value,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transfer_history_record_deserializes_legacy_json_without_relative_paths() {
        let legacy_json = r#"{
            "id": "hist-1",
            "direction": "send",
            "peerId": "peer-1",
            "peerName": "LegacyPeer",
            "fileCount": 1,
            "totalBytes": 42,
            "savedPaths": ["/tmp/file.txt"],
            "status": "completed",
            "errorMessage": null,
            "createdAt": 1000
        }"#;
        let parsed: TransferHistoryRecord = serde_json::from_str(legacy_json).unwrap();
        assert_eq!(parsed.id, "hist-1");
        assert_eq!(parsed.saved_paths, vec!["/tmp/file.txt".to_string()]);
        assert!(parsed.relative_paths.is_empty());
    }
}
