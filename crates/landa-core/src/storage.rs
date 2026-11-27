use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::Utc;
use landa_proto::{DeviceCategory, DeviceOs, PeerId};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::error::CoreError;
use crate::events::{FriendRequestInfo, ManualTarget, TransferHistoryRecord};

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub device_name: String,
    pub download_dir: String,
    pub auto_accept_from_friends: bool,
    #[serde(default = "default_true")]
    pub auto_copy_clipboard: bool,
    pub minimize_to_tray: bool,
    #[serde(default)]
    pub left_handed_mode: bool,
    pub theme: String,
    pub language: String,
    pub discovery_interval_secs: u64,
    pub enabled_modules: HashMap<String, bool>,
    #[serde(default)]
    pub excluded_subnets: Vec<String>,
}

impl AppSettings {
    pub fn default_with(device_name: String, download_dir: PathBuf) -> Self {
        let mut enabled_modules = HashMap::new();
        enabled_modules.insert("shares".to_string(), true);
        enabled_modules.insert("clipboard".to_string(), true);
        enabled_modules.insert("videolink".to_string(), true);
        Self {
            device_name,
            download_dir: download_dir.to_string_lossy().to_string(),
            auto_accept_from_friends: false,
            auto_copy_clipboard: true,
            minimize_to_tray: true,
            left_handed_mode: false,
            theme: "dark".to_string(),
            language: "ru".to_string(),
            discovery_interval_secs: 5,
            enabled_modules,
            excluded_subnets: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct StoredPeer {
    pub peer_id: PeerId,
    pub public_key: [u8; 32],
    pub alias_name: Option<String>,
    pub device_name: String,
    pub os: DeviceOs,
    pub category: DeviceCategory,
    pub is_trusted: bool,
    pub last_ip: Option<String>,
    pub last_port: Option<u16>,
    pub last_seen_at: i64,
}

pub struct Storage {
    conn: Mutex<Connection>,
}

impl Storage {
    pub fn open(db_path: &Path) -> Result<Self, CoreError> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(db_path)?;
        conn.execute_batch(
            "
            PRAGMA journal_mode = WAL;
            PRAGMA foreign_keys = ON;

            CREATE TABLE IF NOT EXISTS peers (
                peer_id TEXT PRIMARY KEY,
                public_key BLOB NOT NULL,
                alias_name TEXT,
                device_name TEXT NOT NULL,
                os TEXT NOT NULL,
                category TEXT NOT NULL,
                is_trusted INTEGER NOT NULL DEFAULT 0,
                last_ip TEXT,
                last_port INTEGER,
                last_seen_at INTEGER NOT NULL,
                created_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS friend_requests (
                peer_id TEXT PRIMARY KEY,
                public_key BLOB NOT NULL,
                device_name TEXT NOT NULL,
                os TEXT NOT NULL,
                category TEXT NOT NULL,
                sas_code TEXT NOT NULL,
                direction TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS transfer_history (
                id TEXT PRIMARY KEY,
                direction TEXT NOT NULL,
                peer_id TEXT NOT NULL,
                peer_name TEXT NOT NULL,
                file_count INTEGER NOT NULL,
                total_bytes INTEGER NOT NULL,
                saved_paths_json TEXT NOT NULL,
                relative_paths_json TEXT NOT NULL DEFAULT '[]',
                status TEXT NOT NULL,
                error_message TEXT,
                created_at INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_transfer_history_created
                ON transfer_history(created_at DESC);

            CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS manual_targets (
                ip TEXT PRIMARY KEY,
                label TEXT,
                created_at INTEGER NOT NULL
            );
            ",
        )?;
        ensure_transfer_history_columns(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn upsert_peer(
        &self,
        peer_id: &PeerId,
        public_key: &[u8; 32],
        device_name: &str,
        os: DeviceOs,
        category: DeviceCategory,
        ip: Option<&str>,
        port: Option<u16>,
    ) -> Result<(), CoreError> {
        let now = Utc::now().timestamp_millis();
        let conn = self.conn.lock().expect("db lock poisoned");
        conn.execute(
            "
            INSERT INTO peers (
                peer_id, public_key, device_name, os, category,
                is_trusted, last_ip, last_port, last_seen_at, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6, ?7, ?8, ?8)
            ON CONFLICT(peer_id) DO UPDATE SET
                public_key = excluded.public_key,
                device_name = excluded.device_name,
                os = excluded.os,
                category = excluded.category,
                last_ip = COALESCE(excluded.last_ip, peers.last_ip),
                last_port = COALESCE(excluded.last_port, peers.last_port),
                last_seen_at = excluded.last_seen_at
            ",
            params![
                peer_id.to_hex(),
                public_key.as_slice(),
                device_name,
                os_to_str(os),
                category_to_str(category),
                ip,
                port.map(i64::from),
                now,
            ],
        )?;
        Ok(())
    }

    pub fn get_peer(&self, peer_id: &PeerId) -> Result<Option<StoredPeer>, CoreError> {
        let conn = self.conn.lock().expect("db lock poisoned");
        conn.query_row(
            "SELECT peer_id, public_key, alias_name, device_name, os, category, is_trusted, last_ip, last_port, last_seen_at
             FROM peers WHERE peer_id = ?1",
            params![peer_id.to_hex()],
            row_to_stored_peer,
        )
        .optional()
        .map_err(CoreError::from)
    }

    pub fn list_peers(&self) -> Result<Vec<StoredPeer>, CoreError> {
        let conn = self.conn.lock().expect("db lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT peer_id, public_key, alias_name, device_name, os, category, is_trusted, last_ip, last_port, last_seen_at
             FROM peers ORDER BY is_trusted DESC, last_seen_at DESC",
        )?;
        let rows = stmt.query_map([], row_to_stored_peer)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn set_peer_trusted(&self, peer_id: &PeerId, trusted: bool) -> Result<(), CoreError> {
        let conn = self.conn.lock().expect("db lock poisoned");
        conn.execute(
            "UPDATE peers SET is_trusted = ?2 WHERE peer_id = ?1",
            params![peer_id.to_hex(), i32::from(trusted)],
        )?;
        Ok(())
    }

    pub fn set_peer_alias(&self, peer_id: &PeerId, alias: Option<&str>) -> Result<(), CoreError> {
        let cleaned = alias.map(str::trim).filter(|s| !s.is_empty());
        let conn = self.conn.lock().expect("db lock poisoned");
        conn.execute(
            "UPDATE peers SET alias_name = ?2 WHERE peer_id = ?1",
            params![peer_id.to_hex(), cleaned],
        )?;
        Ok(())
    }

    pub fn forget_peer(&self, peer_id: &PeerId) -> Result<(), CoreError> {
        let hex = peer_id.to_hex();
        let mut conn = self.conn.lock().expect("db lock poisoned");
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM friend_requests WHERE peer_id = ?1",
            params![&hex],
        )?;
        tx.execute("DELETE FROM peers WHERE peer_id = ?1", params![&hex])?;
        tx.commit()?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn upsert_friend_request(
        &self,
        peer_id: &PeerId,
        public_key: &[u8; 32],
        device_name: &str,
        os: DeviceOs,
        category: DeviceCategory,
        sas_code: &str,
        direction: &str,
    ) -> Result<(), CoreError> {
        let now = Utc::now().timestamp_millis();
        let conn = self.conn.lock().expect("db lock poisoned");
        conn.execute(
            "
            INSERT INTO friend_requests (
                peer_id, public_key, device_name, os, category, sas_code, direction, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            ON CONFLICT(peer_id) DO UPDATE SET
                public_key = excluded.public_key,
                device_name = excluded.device_name,
                os = excluded.os,
                category = excluded.category,
                sas_code = excluded.sas_code,
                direction = excluded.direction,
                created_at = excluded.created_at
            ",
            params![
                peer_id.to_hex(),
                public_key.as_slice(),
                device_name,
                os_to_str(os),
                category_to_str(category),
                sas_code,
                direction,
                now,
            ],
        )?;
        Ok(())
    }

    pub fn remove_friend_request(&self, peer_id: &PeerId) -> Result<(), CoreError> {
        let conn = self.conn.lock().expect("db lock poisoned");
        conn.execute(
            "DELETE FROM friend_requests WHERE peer_id = ?1",
            params![peer_id.to_hex()],
        )?;
        Ok(())
    }

    pub fn list_friend_requests(&self) -> Result<Vec<FriendRequestInfo>, CoreError> {
        let conn = self.conn.lock().expect("db lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT peer_id, device_name, os, category, sas_code, direction, created_at
             FROM friend_requests ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            let os_str: String = row.get(2)?;
            let cat_str: String = row.get(3)?;
            Ok(FriendRequestInfo {
                peer_id: row.get(0)?,
                device_name: row.get(1)?,
                os: str_to_os(&os_str),
                category: str_to_category(&cat_str),
                sas_code: row.get(4)?,
                direction: row.get(5)?,
                created_at: row.get(6)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn insert_transfer_history(&self, record: &TransferHistoryRecord) -> Result<(), CoreError> {
        let paths_json = serde_json::to_string(&record.saved_paths)
            .map_err(|e| CoreError::InvalidInput(e.to_string()))?;
        let rel_paths_json = serde_json::to_string(&record.relative_paths)
            .map_err(|e| CoreError::InvalidInput(e.to_string()))?;
        let conn = self.conn.lock().expect("db lock poisoned");
        conn.execute(
            "
            INSERT INTO transfer_history (
                id, direction, peer_id, peer_name, file_count, total_bytes,
                saved_paths_json, relative_paths_json, status, error_message, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
            ",
            params![
                record.id,
                record.direction,
                record.peer_id,
                record.peer_name,
                record.file_count as i64,
                record.total_bytes as i64,
                paths_json,
                rel_paths_json,
                record.status,
                record.error_message,
                record.created_at,
            ],
        )?;
        Ok(())
    }

    pub fn list_transfer_history(
        &self,
        limit: usize,
    ) -> Result<Vec<TransferHistoryRecord>, CoreError> {
        let conn = self.conn.lock().expect("db lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, direction, peer_id, peer_name, file_count, total_bytes,
                    saved_paths_json, relative_paths_json, status, error_message, created_at
             FROM transfer_history ORDER BY created_at DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            let paths_json: String = row.get(6)?;
            let rel_paths_json: String = row.get(7)?;
            let saved_paths: Vec<String> = serde_json::from_str(&paths_json).unwrap_or_default();
            let relative_paths: Vec<String> =
                serde_json::from_str(&rel_paths_json).unwrap_or_default();
            Ok(TransferHistoryRecord {
                id: row.get(0)?,
                direction: row.get(1)?,
                peer_id: row.get(2)?,
                peer_name: row.get(3)?,
                file_count: row.get::<_, i64>(4)?.max(0) as usize,
                total_bytes: row.get::<_, i64>(5)?.max(0) as u64,
                saved_paths,
                relative_paths,
                status: row.get(8)?,
                error_message: row.get(9)?,
                created_at: row.get(10)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn clear_transfer_history(&self) -> Result<(), CoreError> {
        let conn = self.conn.lock().expect("db lock poisoned");
        conn.execute("DELETE FROM transfer_history", [])?;
        Ok(())
    }

    pub fn load_settings(&self, defaults: AppSettings) -> Result<AppSettings, CoreError> {
        let conn = self.conn.lock().expect("db lock poisoned");
        let raw: Option<String> = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'app_settings'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        match raw {
            Some(json) => Ok(serde_json::from_str(&json).unwrap_or(defaults)),
            None => Ok(defaults),
        }
    }

    pub fn save_settings(&self, settings: &AppSettings) -> Result<(), CoreError> {
        let json =
            serde_json::to_string(settings).map_err(|e| CoreError::InvalidInput(e.to_string()))?;
        let conn = self.conn.lock().expect("db lock poisoned");
        conn.execute(
            "INSERT INTO settings (key, value) VALUES ('app_settings', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![json],
        )?;
        Ok(())
    }

    pub fn get_setting_json<T: for<'de> Deserialize<'de>>(
        &self,
        key: &str,
    ) -> Result<Option<T>, CoreError> {
        let conn = self.conn.lock().expect("db lock poisoned");
        let raw: Option<String> = conn
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()?;
        Ok(raw.and_then(|s| serde_json::from_str(&s).ok()))
    }

    pub fn set_setting_json<T: Serialize>(&self, key: &str, value: &T) -> Result<(), CoreError> {
        let json =
            serde_json::to_string(value).map_err(|e| CoreError::InvalidInput(e.to_string()))?;
        let conn = self.conn.lock().expect("db lock poisoned");
        conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, json],
        )?;
        Ok(())
    }

    pub fn list_manual_targets(&self) -> Result<Vec<ManualTarget>, CoreError> {
        let conn = self.conn.lock().expect("db lock poisoned");
        let mut stmt =
            conn.prepare("SELECT ip, label FROM manual_targets ORDER BY created_at DESC")?;
        let rows = stmt.query_map([], |row| {
            Ok(ManualTarget {
                ip: row.get(0)?,
                label: row.get(1)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn add_manual_target(&self, ip: &str, label: Option<&str>) -> Result<(), CoreError> {
        let now = Utc::now().timestamp_millis();
        let conn = self.conn.lock().expect("db lock poisoned");
        conn.execute(
            "INSERT INTO manual_targets (ip, label, created_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(ip) DO UPDATE SET label = excluded.label",
            params![ip, label, now],
        )?;
        Ok(())
    }

    pub fn remove_manual_target(&self, ip: &str) -> Result<(), CoreError> {
        let conn = self.conn.lock().expect("db lock poisoned");
        conn.execute("DELETE FROM manual_targets WHERE ip = ?1", params![ip])?;
        Ok(())
    }
}

fn ensure_transfer_history_columns(conn: &Connection) -> Result<(), CoreError> {
    let mut stmt = conn.prepare("PRAGMA table_info(transfer_history)")?;
    let cols = stmt.query_map([], |row| row.get::<_, String>(1))?;
    let mut has_relative_paths = false;
    for col in cols {
        if col? == "relative_paths_json" {
            has_relative_paths = true;
            break;
        }
    }
    if !has_relative_paths {
        conn.execute(
            "ALTER TABLE transfer_history ADD COLUMN relative_paths_json TEXT NOT NULL DEFAULT '[]'",
            [],
        )?;
    }
    Ok(())
}

fn row_to_stored_peer(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredPeer> {
    let peer_id_hex: String = row.get(0)?;
    let pk_blob: Vec<u8> = row.get(1)?;
    let os_str: String = row.get(4)?;
    let cat_str: String = row.get(5)?;
    let peer_id = PeerId::from_hex(&peer_id_hex).unwrap_or(PeerId([0u8; 16]));
    let mut public_key = [0u8; 32];
    if pk_blob.len() == 32 {
        public_key.copy_from_slice(&pk_blob);
    }
    Ok(StoredPeer {
        peer_id,
        public_key,
        alias_name: row.get(2)?,
        device_name: row.get(3)?,
        os: str_to_os(&os_str),
        category: str_to_category(&cat_str),
        is_trusted: row.get::<_, i32>(6)? != 0,
        last_ip: row.get(7)?,
        last_port: row.get::<_, Option<i64>>(8)?.map(|p| p as u16),
        last_seen_at: row.get(9)?,
    })
}

fn os_to_str(os: DeviceOs) -> &'static str {
    match os {
        DeviceOs::Windows => "windows",
        DeviceOs::Linux => "linux",
        DeviceOs::MacOs => "macos",
        DeviceOs::Android => "android",
        DeviceOs::Ios => "ios",
        DeviceOs::Unknown => "unknown",
    }
}

fn str_to_os(s: &str) -> DeviceOs {
    match s {
        "windows" => DeviceOs::Windows,
        "linux" => DeviceOs::Linux,
        "macos" => DeviceOs::MacOs,
        "android" => DeviceOs::Android,
        "ios" => DeviceOs::Ios,
        _ => DeviceOs::Unknown,
    }
}

fn category_to_str(cat: DeviceCategory) -> &'static str {
    match cat {
        DeviceCategory::Desktop => "desktop",
        DeviceCategory::Phone => "phone",
        DeviceCategory::Unknown => "unknown",
    }
}

fn str_to_category(s: &str) -> DeviceCategory {
    match s {
        "desktop" => DeviceCategory::Desktop,
        "phone" => DeviceCategory::Phone,
        _ => DeviceCategory::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_settings_deserializes_legacy_json_without_excluded_subnets() {
        let legacy_json = r#"{
            "deviceName": "Legacy PC",
            "downloadDir": "/tmp/downloads",
            "autoAcceptFromFriends": true,
            "minimizeToTray": false,
            "leftHandedMode": false,
            "theme": "dark",
            "language": "ru",
            "discoveryIntervalSecs": 5,
            "enabledModules": {"shares": true}
        }"#;
        let parsed: AppSettings = serde_json::from_str(legacy_json).unwrap();
        assert_eq!(parsed.device_name, "Legacy PC");
        assert!(parsed.excluded_subnets.is_empty());
        assert!(parsed.auto_copy_clipboard);
    }

    #[test]
    fn storage_persists_excluded_subnets_roundtrip() {
        let temp_dir = std::env::temp_dir().join(format!("landa-storage-{}", uuid::Uuid::new_v4()));
        let db_path = temp_dir.join("landa.sqlite");
        let storage = Storage::open(&db_path).unwrap();

        let mut settings = AppSettings::default_with("Test".into(), temp_dir.join("dl"));
        settings.excluded_subnets = vec!["192.168.56.0/24".into(), "172.17.0.0/16".into()];
        settings.auto_copy_clipboard = false;
        storage.save_settings(&settings).unwrap();

        let loaded = storage
            .load_settings(AppSettings::default_with(
                "Fallback".into(),
                temp_dir.clone(),
            ))
            .unwrap();
        assert_eq!(
            loaded.excluded_subnets,
            vec!["192.168.56.0/24".to_string(), "172.17.0.0/16".to_string()]
        );
        assert!(!loaded.auto_copy_clipboard);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn storage_forget_peer_removes_peer_and_friend_requests() {
        let temp_dir = std::env::temp_dir().join(format!("landa-forget-{}", uuid::Uuid::new_v4()));
        let db_path = temp_dir.join("landa.sqlite");
        let storage = Storage::open(&db_path).unwrap();

        let pk_a = [1u8; 32];
        let pk_b = [2u8; 32];
        let peer_a = PeerId::from_public_key(&pk_a);
        let peer_b = PeerId::from_public_key(&pk_b);

        storage
            .upsert_peer(
                &peer_a,
                &pk_a,
                "Device A",
                DeviceOs::Android,
                DeviceCategory::Phone,
                Some("192.168.0.10"),
                Some(40406),
            )
            .unwrap();
        storage.set_peer_trusted(&peer_a, true).unwrap();
        storage.set_peer_alias(&peer_a, Some("My Phone")).unwrap();
        storage
            .upsert_friend_request(
                &peer_a,
                &pk_a,
                "Device A",
                DeviceOs::Android,
                DeviceCategory::Phone,
                "123456",
                "incoming",
            )
            .unwrap();

        storage
            .upsert_peer(
                &peer_b,
                &pk_b,
                "Device B",
                DeviceOs::Windows,
                DeviceCategory::Desktop,
                Some("192.168.0.11"),
                Some(40406),
            )
            .unwrap();
        storage
            .upsert_friend_request(
                &peer_b,
                &pk_b,
                "Device B",
                DeviceOs::Windows,
                DeviceCategory::Desktop,
                "654321",
                "outgoing",
            )
            .unwrap();

        storage.forget_peer(&peer_a).unwrap();

        assert!(storage.get_peer(&peer_a).unwrap().is_none());
        let peers = storage.list_peers().unwrap();
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].peer_id, peer_b);

        let reqs = storage.list_friend_requests().unwrap();
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].peer_id, peer_b.to_hex());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn storage_migrates_legacy_transfer_history_and_persists_relative_paths() {
        let temp_dir =
            std::env::temp_dir().join(format!("landa-hist-mig-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp_dir).unwrap();
        let db_path = temp_dir.join("landa.sqlite");

        // Create legacy transfer_history table without relative_paths_json
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "
                CREATE TABLE transfer_history (
                    id TEXT PRIMARY KEY,
                    direction TEXT NOT NULL,
                    peer_id TEXT NOT NULL,
                    peer_name TEXT NOT NULL,
                    file_count INTEGER NOT NULL,
                    total_bytes INTEGER NOT NULL,
                    saved_paths_json TEXT NOT NULL,
                    status TEXT NOT NULL,
                    error_message TEXT,
                    created_at INTEGER NOT NULL
                );
                INSERT INTO transfer_history (
                    id, direction, peer_id, peer_name, file_count, total_bytes,
                    saved_paths_json, status, error_message, created_at
                ) VALUES (
                    'legacy-1', 'send', 'peer-1', 'LegacyPeer', 1, 100,
                    '[\"/tmp/old.txt\"]', 'completed', NULL, 1000
                );
                ",
            )
            .unwrap();
        }

        let storage = Storage::open(&db_path).unwrap();
        storage
            .insert_transfer_history(&TransferHistoryRecord {
                id: "new-1".into(),
                direction: "send".into(),
                peer_id: "peer-2".into(),
                peer_name: "AndroidPeer".into(),
                file_count: 2,
                total_bytes: 200,
                saved_paths: vec![
                    "content://provider/doc/1".into(),
                    "content://provider/doc/2".into(),
                ],
                relative_paths: vec!["a/report.pdf".into(), "b/report.pdf".into()],
                status: "completed".into(),
                error_message: None,
                created_at: 2000,
            })
            .unwrap();

        let history = storage.list_transfer_history(10).unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].id, "new-1");
        assert_eq!(
            history[0].relative_paths,
            vec!["a/report.pdf".to_string(), "b/report.pdf".to_string()]
        );
        assert_eq!(history[1].id, "legacy-1");
        assert!(history[1].relative_paths.is_empty());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
