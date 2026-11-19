//! Everywhere but Android: documents are paths, so the Android file plugin's
//! commands have nothing to do. Same API as `android.rs`.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Runtime};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Opened {
    pub uri: String,
    pub path: Option<String>,
}

pub struct Content {
    pub name: String,
    pub text: String,
    pub lossy: bool,
}

const ANDROID_ONLY: &str = "content:// documents are only supported on Android";

pub async fn read<R: Runtime>(_: &AppHandle<R>, _: &str) -> Result<Content, String> {
    Err(ANDROID_ONLY.into())
}

pub async fn write<R: Runtime>(_: &AppHandle<R>, _: &str, _: &str) -> Result<(), String> {
    Err(ANDROID_ONLY.into())
}

pub async fn pending<R: Runtime>(_: &AppHandle<R>) -> Result<Vec<Opened>, String> {
    Ok(Vec::new())
}

pub async fn pick_files<R: Runtime>(_: &AppHandle<R>) -> Result<Vec<Opened>, String> {
    Err(ANDROID_ONLY.into())
}

pub async fn pick_folder<R: Runtime>(_: &AppHandle<R>) -> Result<Option<Opened>, String> {
    Err(ANDROID_ONLY.into())
}

pub async fn path_of<R: Runtime>(_: &AppHandle<R>, _: &str) -> Result<Option<String>, String> {
    Ok(None)
}

pub async fn all_files<R: Runtime>(_: &AppHandle<R>) -> Result<bool, String> {
    Ok(true)
}

pub async fn request_all_files<R: Runtime>(_: &AppHandle<R>) -> Result<bool, String> {
    Ok(true)
}

pub async fn move_to_back<R: Runtime>(_: &AppHandle<R>) -> Result<(), String> {
    Ok(())
}
