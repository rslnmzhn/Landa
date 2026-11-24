use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use landa_proto::path_policy::validate_relative_path;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::AsyncWriteExt;

const GITHUB_LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/rslnmzhn/Landa/releases/latest";
const USER_AGENT: &str = concat!("Landa/", env!("CARGO_PKG_VERSION"));
const MAX_MANIFEST_BYTES: usize = 512 * 1024;
const MAX_UPDATE_FILE_BYTES: u64 = 512 * 1024 * 1024;
const PROGRESS_EMIT_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateAssetInfo {
    pub platform: String,
    pub arch: String,
    pub format: String,
    pub file_name: String,
    pub size: u64,
    pub sha256: String,
    pub download_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheckResult {
    pub current_version: String,
    pub latest_version: String,
    pub has_update: bool,
    pub release_tag: String,
    pub release_notes: String,
    pub published_at: Option<String>,
    pub asset: Option<UpdateAssetInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProgressEvent {
    pub stage: String,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub file_path: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GithubReleaseDto {
    tag_name: String,
    body: Option<String>,
    published_at: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<GithubAssetDto>,
}

#[derive(Debug, Deserialize)]
struct GithubAssetDto {
    name: String,
    browser_download_url: String,
    size: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReleaseManifestDto {
    release: ManifestReleaseDto,
    assets: Vec<ManifestAssetDto>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManifestReleaseDto {
    tag: String,
    version: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManifestAssetDto {
    platform: String,
    arch: String,
    format: String,
    #[serde(default)]
    primary: bool,
    file_name: String,
    size: u64,
    sha256: String,
    download_url: String,
}

fn build_http_client() -> Result<reqwest::Client, String> {
    let mut root_store = rustls::RootCertStore::empty();
    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let tls_config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| format!("TLS config error: {e}"))?
        .with_root_certificates(root_store)
        .with_no_client_auth();

    reqwest::Client::builder()
        .use_preconfigured_tls(tls_config)
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(300))
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))
}

pub async fn check_for_updates() -> Result<UpdateCheckResult, String> {
    let client = build_http_client()?;
    let resp = client
        .get(GITHUB_LATEST_RELEASE_URL)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("Failed to reach GitHub Releases: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("GitHub API returned HTTP {}", resp.status()));
    }

    let release: GithubReleaseDto = resp
        .json()
        .await
        .map_err(|e| format!("Invalid GitHub release JSON: {e}"))?;

    if release.draft || release.prerelease {
        let current = env!("CARGO_PKG_VERSION").to_string();
        return Ok(UpdateCheckResult {
            current_version: current.clone(),
            latest_version: current,
            has_update: false,
            release_tag: release.tag_name,
            release_notes: String::new(),
            published_at: release.published_at,
            asset: None,
        });
    }

    let current_version = env!("CARGO_PKG_VERSION").to_string();
    let manifest_asset = release
        .assets
        .iter()
        .find(|a| a.name.ends_with("-release-manifest.json"));

    if let Some(manifest_dto) = manifest_asset {
        if manifest_dto.size > MAX_MANIFEST_BYTES as u64 {
            return Err("Release manifest exceeds size limit".into());
        }
        validate_download_url(&manifest_dto.browser_download_url)?;

        let manifest_resp = client
            .get(&manifest_dto.browser_download_url)
            .send()
            .await
            .map_err(|e| format!("Failed to download release manifest: {e}"))?;
        if !manifest_resp.status().is_success() {
            return Err(format!(
                "Manifest download returned HTTP {}",
                manifest_resp.status()
            ));
        }
        let bytes = manifest_resp
            .bytes()
            .await
            .map_err(|e| format!("Failed to read manifest body: {e}"))?;
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err("Release manifest exceeds size limit".into());
        }

        let manifest: ReleaseManifestDto = serde_json::from_slice(&bytes)
            .map_err(|e| format!("Invalid release manifest JSON: {e}"))?;

        let latest_version = manifest.release.version.trim().to_string();
        let has_update = is_newer_version(&latest_version, &current_version);
        let selected = select_asset_for_current_platform(&manifest.assets);

        return Ok(UpdateCheckResult {
            current_version,
            latest_version,
            has_update,
            release_tag: manifest.release.tag,
            release_notes: release.body.unwrap_or_default().trim().to_string(),
            published_at: release.published_at,
            asset: selected,
        });
    }

    let latest_version = release
        .tag_name
        .trim()
        .trim_start_matches(['v', 'V'])
        .to_string();
    let has_update = is_newer_version(&latest_version, &current_version);

    Ok(UpdateCheckResult {
        current_version,
        latest_version,
        has_update,
        release_tag: release.tag_name,
        release_notes: release.body.unwrap_or_default().trim().to_string(),
        published_at: release.published_at,
        asset: None,
    })
}

pub async fn download_and_verify_update(
    app: &AppHandle,
    asset: UpdateAssetInfo,
) -> Result<String, String> {
    validate_download_url(&asset.download_url)?;
    let safe_name = sanitize_update_filename(&asset.file_name)?;
    let expected_sha = normalize_sha256(&asset.sha256)?;

    if asset.size > MAX_UPDATE_FILE_BYTES {
        return Err("Update file exceeds maximum allowed size".into());
    }

    let updates_dir = resolve_updates_dir(app)?;
    tokio::fs::create_dir_all(&updates_dir)
        .await
        .map_err(|e| format!("Failed to create updates directory: {e}"))?;

    let target_path = updates_dir.join(&safe_name);
    if target_path.exists() {
        if let Ok(existing_sha) = compute_file_sha256(&target_path).await {
            if existing_sha == expected_sha {
                let path_str = target_path.to_string_lossy().to_string();
                emit_progress(
                    app,
                    "ready",
                    asset.size,
                    asset.size,
                    Some(path_str.clone()),
                    None,
                );
                return Ok(path_str);
            }
        }
        let _ = tokio::fs::remove_file(&target_path).await;
    }

    let part_path = updates_dir.join(format!("{safe_name}.part"));
    let _ = tokio::fs::remove_file(&part_path).await;

    emit_progress(app, "downloading", 0, asset.size, None, None);

    let client = build_http_client()?;
    let mut resp = client.get(&asset.download_url).send().await.map_err(|e| {
        let msg = format!("Download failed: {e}");
        emit_progress(app, "failed", 0, asset.size, None, Some(msg.clone()));
        msg
    })?;

    if !resp.status().is_success() {
        let msg = format!("Download returned HTTP {}", resp.status());
        emit_progress(app, "failed", 0, asset.size, None, Some(msg.clone()));
        return Err(msg);
    }

    let total_bytes = resp.content_length().unwrap_or(asset.size);
    if total_bytes > MAX_UPDATE_FILE_BYTES {
        let msg = "Update file exceeds maximum allowed size".to_string();
        emit_progress(app, "failed", 0, total_bytes, None, Some(msg.clone()));
        return Err(msg);
    }

    let mut file = tokio::fs::File::create(&part_path)
        .await
        .map_err(|e| format!("Failed to create temp update file: {e}"))?;
    let mut hasher = Sha256::new();
    let mut downloaded: u64 = 0;
    let mut last_emit = Instant::now();

    loop {
        let chunk = match resp.chunk().await {
            Ok(Some(c)) => c,
            Ok(None) => break,
            Err(e) => {
                let _ = tokio::fs::remove_file(&part_path).await;
                let msg = format!("Download stream error: {e}");
                emit_progress(
                    app,
                    "failed",
                    downloaded,
                    total_bytes,
                    None,
                    Some(msg.clone()),
                );
                return Err(msg);
            }
        };

        downloaded = downloaded.saturating_add(chunk.len() as u64);
        if downloaded > MAX_UPDATE_FILE_BYTES {
            let _ = tokio::fs::remove_file(&part_path).await;
            let msg = "Update file exceeded maximum allowed size".to_string();
            emit_progress(
                app,
                "failed",
                downloaded,
                total_bytes,
                None,
                Some(msg.clone()),
            );
            return Err(msg);
        }

        hasher.update(&chunk);
        if let Err(e) = file.write_all(&chunk).await {
            let _ = tokio::fs::remove_file(&part_path).await;
            let msg = format!("Failed to write update file: {e}");
            emit_progress(
                app,
                "failed",
                downloaded,
                total_bytes,
                None,
                Some(msg.clone()),
            );
            return Err(msg);
        }

        if last_emit.elapsed() >= PROGRESS_EMIT_INTERVAL {
            last_emit = Instant::now();
            emit_progress(app, "downloading", downloaded, total_bytes, None, None);
        }
    }

    file.flush()
        .await
        .map_err(|e| format!("Failed to flush update file: {e}"))?;
    drop(file);

    emit_progress(app, "verifying", downloaded, total_bytes, None, None);

    if asset.size > 0 && downloaded != asset.size {
        let _ = tokio::fs::remove_file(&part_path).await;
        let msg = format!(
            "Downloaded size mismatch: expected {} bytes, got {}",
            asset.size, downloaded
        );
        emit_progress(
            app,
            "failed",
            downloaded,
            total_bytes,
            None,
            Some(msg.clone()),
        );
        return Err(msg);
    }

    let actual_sha = hex_encode(&hasher.finalize());
    if actual_sha != expected_sha {
        let _ = tokio::fs::remove_file(&part_path).await;
        let msg = "SHA-256 verification failed for downloaded update".to_string();
        emit_progress(
            app,
            "failed",
            downloaded,
            total_bytes,
            None,
            Some(msg.clone()),
        );
        return Err(msg);
    }

    tokio::fs::rename(&part_path, &target_path)
        .await
        .map_err(|e| format!("Failed to finalize update file: {e}"))?;

    let path_str = target_path.to_string_lossy().to_string();
    emit_progress(
        app,
        "ready",
        downloaded,
        total_bytes,
        Some(path_str.clone()),
        None,
    );
    Ok(path_str)
}

fn resolve_verified_update_file(updates_dir: &Path, file_path: &str) -> Result<PathBuf, String> {
    let canonical_dir = updates_dir
        .canonicalize()
        .map_err(|e| format!("Updates directory unavailable: {e}"))?;
    let canonical_file = PathBuf::from(file_path)
        .canonicalize()
        .map_err(|e| format!("Update file not found: {e}"))?;

    if !canonical_file.starts_with(&canonical_dir) || !canonical_file.is_file() {
        return Err("Invalid update file path".into());
    }
    Ok(canonical_file)
}

pub async fn install_update(app: &AppHandle, file_path: &str) -> Result<(), String> {
    let updates_dir = resolve_updates_dir(app)?;
    let canonical_file = resolve_verified_update_file(&updates_dir, file_path)?;

    emit_progress(
        app,
        "installing",
        0,
        0,
        Some(canonical_file.to_string_lossy().to_string()),
        None,
    );

    #[cfg(target_os = "windows")]
    {
        let ext = canonical_file
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if ext == "exe" {
            std::process::Command::new(&canonical_file)
                .spawn()
                .map_err(|e| format!("Failed to launch installer: {e}"))?;
            let app_handle = app.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(Duration::from_millis(350)).await;
                app_handle.exit(0);
            });
            return Ok(());
        }
        use tauri_plugin_opener::OpenerExt;
        app.opener()
            .reveal_item_in_dir(&canonical_file)
            .map_err(|e| format!("Failed to open update folder: {e}"))?;
        Ok(())
    }

    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::PermissionsExt;
        let ext = canonical_file
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if ext == "appimage" {
            let perms = std::fs::Permissions::from_mode(0o755);
            std::fs::set_permissions(&canonical_file, perms)
                .map_err(|e| format!("Failed to mark AppImage executable: {e}"))?;
            std::process::Command::new(&canonical_file)
                .spawn()
                .map_err(|e| format!("Failed to launch AppImage: {e}"))?;
            let app_handle = app.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(Duration::from_millis(350)).await;
                app_handle.exit(0);
            });
            return Ok(());
        }
        use tauri_plugin_opener::OpenerExt;
        app.opener()
            .reveal_item_in_dir(&canonical_file)
            .map_err(|e| format!("Failed to open update folder: {e}"))?;
        Ok(())
    }

    #[cfg(target_os = "android")]
    {
        if let Some(bridge) = app.try_state::<crate::android_bridge::AndroidBridge<tauri::Wry>>() {
            bridge
                .install_apk_update_async(&canonical_file.to_string_lossy())
                .await
        } else {
            Err("Android bridge unavailable".into())
        }
    }

    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "android")))]
    {
        use tauri_plugin_opener::OpenerExt;
        app.opener()
            .reveal_item_in_dir(&canonical_file)
            .map_err(|e| format!("Failed to open update folder: {e}"))?;
        Ok(())
    }
}

fn resolve_updates_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let base = app
        .path()
        .app_cache_dir()
        .or_else(|_| app.path().app_data_dir())
        .map_err(|e| format!("Failed to resolve cache directory: {e}"))?;
    Ok(base.join("updates"))
}

fn emit_progress(
    app: &AppHandle,
    stage: &str,
    downloaded_bytes: u64,
    total_bytes: u64,
    file_path: Option<String>,
    error: Option<String>,
) {
    let _ = app.emit(
        "landa://update-progress",
        UpdateProgressEvent {
            stage: stage.to_string(),
            downloaded_bytes,
            total_bytes,
            file_path,
            error,
        },
    );
}

fn select_asset_for_current_platform(assets: &[ManifestAssetDto]) -> Option<UpdateAssetInfo> {
    let (platform, preferred_arches, preferred_formats): (&str, &[&str], &[&str]) =
        if cfg!(target_os = "windows") {
            let arch = if cfg!(target_arch = "x86_64") {
                &["x86_64"][..]
            } else {
                &[std::env::consts::ARCH][..]
            };
            ("windows", arch, &["exe", "zip"])
        } else if cfg!(target_os = "linux") {
            let arch = if cfg!(target_arch = "x86_64") {
                &["x86_64"][..]
            } else {
                &[std::env::consts::ARCH][..]
            };
            ("linux", arch, &["appimage", "deb", "zip"])
        } else if cfg!(target_os = "android") {
            let arches: &[&str] = if cfg!(target_arch = "aarch64") {
                &["arm64-v8a", "universal"]
            } else if cfg!(target_arch = "arm") {
                &["armeabi-v7a", "universal"]
            } else if cfg!(target_arch = "x86_64") {
                &["x86_64", "universal"]
            } else {
                &["universal"]
            };
            ("android", arches, &["apk"])
        } else {
            return None;
        };

    for arch in preferred_arches {
        for fmt in preferred_formats {
            if let Some(found) = assets.iter().find(|a| {
                a.platform.eq_ignore_ascii_case(platform)
                    && a.arch.eq_ignore_ascii_case(arch)
                    && a.format.eq_ignore_ascii_case(fmt)
            }) {
                return Some(to_asset_info(found));
            }
        }
    }

    assets
        .iter()
        .find(|a| a.platform.eq_ignore_ascii_case(platform) && a.primary)
        .map(to_asset_info)
}

fn to_asset_info(a: &ManifestAssetDto) -> UpdateAssetInfo {
    UpdateAssetInfo {
        platform: a.platform.clone(),
        arch: a.arch.clone(),
        format: a.format.clone(),
        file_name: a.file_name.clone(),
        size: a.size,
        sha256: a.sha256.clone(),
        download_url: a.download_url.clone(),
    }
}

fn validate_download_url(raw: &str) -> Result<(), String> {
    let parsed = reqwest::Url::parse(raw).map_err(|_| "Invalid update URL".to_string())?;
    if parsed.scheme() != "https" {
        return Err("Update URL must use HTTPS".into());
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| "Update URL missing host".to_string())?
        .to_ascii_lowercase();
    let allowed = host == "github.com"
        || host.ends_with(".github.com")
        || host == "githubusercontent.com"
        || host.ends_with(".githubusercontent.com");
    if !allowed {
        return Err(format!("Untrusted update host: {host}"));
    }
    Ok(())
}

fn sanitize_update_filename(raw: &str) -> Result<String, String> {
    let rel = validate_relative_path(raw).map_err(|e| format!("Unsafe update filename: {e}"))?;
    let mut comps = rel.components();
    match (comps.next(), comps.next()) {
        (Some(Component::Normal(name)), None) => Ok(name.to_string_lossy().to_string()),
        _ => Err("Update filename must be a single file name".into()),
    }
}

fn normalize_sha256(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim().to_ascii_lowercase();
    if trimmed.len() != 64 || !trimmed.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Invalid SHA-256 checksum in manifest".into());
    }
    Ok(trimmed)
}

async fn compute_file_sha256(path: &Path) -> Result<String, String> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        use std::io::Read;
        let mut file = std::fs::File::open(&path).map_err(|e| e.to_string())?;
        let mut hasher = Sha256::new();
        let mut buf = [0u8; 64 * 1024];
        loop {
            let n = file.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        Ok(hex_encode(&hasher.finalize()))
    })
    .await
    .map_err(|e| e.to_string())?
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

pub fn parse_semver(raw: &str) -> Option<(u64, u64, u64)> {
    let cleaned = raw
        .trim()
        .trim_start_matches(['v', 'V'])
        .split(['-', '+'])
        .next()?;
    let mut parts = cleaned.split('.');
    let major = parts.next()?.parse::<u64>().ok()?;
    let minor = parts.next()?.parse::<u64>().ok()?;
    let patch = parts.next()?.parse::<u64>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

pub fn is_newer_version(latest: &str, current: &str) -> bool {
    match (parse_semver(latest), parse_semver(current)) {
        (Some(l), Some(c)) => l > c,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_comparison_works() {
        assert!(is_newer_version("0.5.1", "0.5.0"));
        assert!(is_newer_version("v0.6.0", "0.5.9"));
        assert!(!is_newer_version("0.5.0", "0.5.0"));
        assert!(!is_newer_version("0.4.9", "0.5.0"));
    }

    #[test]
    fn rejects_unsafe_urls_and_filenames() {
        assert!(validate_download_url("http://github.com/a/b").is_err());
        assert!(validate_download_url("https://evil.example.com/a.exe").is_err());
        assert!(validate_download_url(
            "https://github.com/rslnmzhn/Landa/releases/download/v0.5.1/landa.exe"
        )
        .is_ok());
        assert!(sanitize_update_filename("../evil.exe").is_err());
        assert!(sanitize_update_filename("sub/evil.exe").is_err());
        assert!(sanitize_update_filename("landa-v0.5.1-windows-x64-setup.exe").is_ok());
    }

    #[test]
    fn resolve_verified_update_file_enforces_updates_dir_containment() {
        let base = std::env::temp_dir().join(format!(
            "landa-updater-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let updates_dir = base.join("updates");
        std::fs::create_dir_all(&updates_dir).unwrap();
        let valid_apk = updates_dir.join("landa-v0.5.1-android-arm64-v8a.apk");
        std::fs::write(&valid_apk, b"apk").unwrap();
        let outside_apk = base.join("outside.apk");
        std::fs::write(&outside_apk, b"evil").unwrap();

        let resolved = resolve_verified_update_file(&updates_dir, &valid_apk.to_string_lossy());
        assert!(resolved.is_ok());
        assert!(
            resolve_verified_update_file(&updates_dir, &outside_apk.to_string_lossy()).is_err()
        );
        assert!(
            resolve_verified_update_file(&updates_dir, &updates_dir.to_string_lossy()).is_err()
        );

        let _ = std::fs::remove_dir_all(&base);
    }
}
