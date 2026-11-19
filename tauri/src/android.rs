//! Android: documents shared by other apps or chosen in the system picker
//! arrive as `content://` URIs, which only the platform can read. The Kotlin
//! side is `FilesPlugin.kt` in the Android project (`gen/android`); these
//! are its Rust ends, called by the app's commands in `lib.rs`.
//!
//! Where a URI is really a file on shared storage and the app has "All files
//! access", the plugin also gives its path. A path is better than the URI:
//! relative images, links to other documents, folders and reloading on change
//! all work from it.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::plugin::{Builder, PluginHandle, TauriPlugin};
use tauri::{AppHandle, Manager, Runtime};

struct Files<R: Runtime>(PluginHandle<R>);

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("mdfiles")
        .setup(|app, api| {
            let handle = api.register_android_plugin("io.github.mdreader.MdReader", "FilesPlugin")?;
            app.manage(Files(handle));
            Ok(())
        })
        .build()
}

/// A document handed to the app: its URI, and its path when it has one the
/// app may read.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Opened {
    pub uri: String,
    pub path: Option<String>,
}

#[derive(Deserialize)]
pub struct Content {
    pub name: String,
    pub text: String,
    pub lossy: bool,
}

#[derive(Deserialize)]
struct Items {
    items: Vec<Opened>,
}

#[derive(Deserialize)]
struct Picked {
    item: Option<Opened>,
}

#[derive(Deserialize)]
struct Granted {
    granted: bool,
}

async fn call<R: Runtime, T: DeserializeOwned>(
    app: &AppHandle<R>,
    command: &str,
    payload: serde_json::Value,
) -> Result<T, String> {
    let files = app.try_state::<Files<R>>().ok_or("the Android file plugin is not loaded")?;
    files.0.run_mobile_plugin_async(command, payload).await.map_err(|e| match e {
        // The plugin's own message ("it is not a text file") reads better
        // than the error's debug form.
        tauri::plugin::mobile::PluginInvokeError::InvokeRejected(r) => r.message.unwrap_or_else(|| "failed".into()),
        e => e.to_string(),
    })
}

pub async fn read<R: Runtime>(app: &AppHandle<R>, uri: &str) -> Result<Content, String> {
    call(app, "read", json!({ "uri": uri })).await
}

pub async fn write<R: Runtime>(app: &AppHandle<R>, uri: &str, text: &str) -> Result<(), String> {
    call::<R, serde_json::Value>(app, "write", json!({ "uri": uri, "text": text })).await.map(drop)
}

/// Documents other apps have sent since the last call (once each).
pub async fn pending<R: Runtime>(app: &AppHandle<R>) -> Result<Vec<Opened>, String> {
    call::<R, Items>(app, "pending", json!({})).await.map(|r| r.items)
}

/// The system's document picker; empty when cancelled.
pub async fn pick_files<R: Runtime>(app: &AppHandle<R>) -> Result<Vec<Opened>, String> {
    call::<R, Items>(app, "pickFiles", json!({})).await.map(|r| r.items)
}

/// The system's folder picker; None when cancelled.
pub async fn pick_folder<R: Runtime>(app: &AppHandle<R>) -> Result<Option<Opened>, String> {
    call::<R, Picked>(app, "pickFolder", json!({})).await.map(|r| r.item)
}

/// The file behind a URI, if the app may read it (after "All files access"
/// was granted to a document that came as a URI).
pub async fn path_of<R: Runtime>(app: &AppHandle<R>, uri: &str) -> Result<Option<String>, String> {
    #[derive(Deserialize)]
    struct Resolved {
        path: Option<String>,
    }
    call::<R, Resolved>(app, "resolve", json!({ "uri": uri })).await.map(|r| r.path)
}

/// Whether the app has "All files access".
pub async fn all_files<R: Runtime>(app: &AppHandle<R>) -> Result<bool, String> {
    call::<R, Granted>(app, "allFiles", json!({})).await.map(|r| r.granted)
}

/// Opens the system setting for "All files access"; true once granted.
pub async fn request_all_files<R: Runtime>(app: &AppHandle<R>) -> Result<bool, String> {
    call::<R, Granted>(app, "requestAllFiles", json!({})).await.map(|r| r.granted)
}

/// Back with nothing left to go back to: the app goes to the background, as
/// Android apps do, rather than ending.
pub async fn move_to_back<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    call::<R, serde_json::Value>(app, "moveToBack", json!({})).await.map(drop)
}
