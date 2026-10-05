//! Update channels. Stable follows the latest GitHub release; beta follows a fixed
//! `updater-beta` release that CI rewrites on every push to main (and every stable release).

use std::sync::Mutex;

use serde::Serialize;
use tauri::ipc::Channel;
use tauri::{AppHandle, State, Url};
use tauri_plugin_updater::{Update, UpdaterExt};

const STABLE_ENDPOINT: &str = "https://github.com/parevo/kiyi/releases/latest/download/latest.json";
const BETA_ENDPOINT: &str = "https://github.com/parevo/kiyi/releases/download/updater-beta/latest.json";

/// The update found by the last check and, once downloaded, its verified bytes.
#[derive(Default)]
pub struct PendingUpdate(Mutex<Option<(Update, Option<Vec<u8>>)>>);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    version: String,
    current_version: String,
    notes: Option<String>,
    date: Option<String>,
    /// Set by adding `"critical": true` to latest.json for security fixes.
    critical: bool,
}

#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum DownloadEvent {
    #[serde(rename_all = "camelCase")]
    Progress { downloaded: u64, total: Option<u64> },
}

#[tauri::command]
pub async fn check_update(app: AppHandle, pending: State<'_, PendingUpdate>, channel: String) -> Result<Option<UpdateInfo>, String> {
    let endpoint = if channel == "beta" { BETA_ENDPOINT } else { STABLE_ENDPOINT };
    let update = app
        .updater_builder()
        .endpoints(vec![Url::parse(endpoint).map_err(|e| e.to_string())?])
        .map_err(|e| e.to_string())?
        .build()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| e.to_string())?;

    let info = update.as_ref().map(|u| UpdateInfo {
        version: u.version.clone(),
        current_version: u.current_version.clone(),
        notes: u.body.clone(),
        date: u.date.map(|d| d.to_string()),
        critical: u.raw_json.get("critical").and_then(|v| v.as_bool()).unwrap_or(false),
    });
    let mut slot = pending.0.lock().unwrap();
    // Keep an already-downloaded payload if the same version is offered again.
    let same = matches!((&*slot, &update), (Some((old, Some(_))), Some(new)) if old.version == new.version);
    if !same {
        *slot = update.map(|u| (u, None));
    }
    Ok(info)
}

/// Downloads (and signature-checks) the pending update in the background. Nothing is installed yet.
#[tauri::command]
pub async fn download_update(pending: State<'_, PendingUpdate>, on_event: Channel<DownloadEvent>) -> Result<(), String> {
    let update = match &*pending.0.lock().unwrap() {
        Some((_, Some(_))) => return Ok(()),
        Some((u, None)) => u.clone(),
        None => return Err("Bekleyen güncelleme yok".into()),
    };
    let mut downloaded = 0u64;
    let bytes = update
        .download(
            move |chunk, total| {
                downloaded += chunk as u64;
                let _ = on_event.send(DownloadEvent::Progress { downloaded, total });
            },
            || {},
        )
        .await
        .map_err(|e| e.to_string())?;
    if let Some((u, slot)) = &mut *pending.0.lock().unwrap() {
        if u.version == update.version {
            *slot = Some(bytes);
        }
    }
    Ok(())
}

/// Installs the downloaded update. The UI relaunches the app right after.
#[tauri::command]
pub fn install_update(pending: State<'_, PendingUpdate>) -> Result<(), String> {
    let (update, bytes) = pending.0.lock().unwrap().take().ok_or("Bekleyen güncelleme yok")?;
    let bytes = bytes.ok_or("Güncelleme henüz indirilmedi")?;
    update.install(bytes).map_err(|e| e.to_string())
}
