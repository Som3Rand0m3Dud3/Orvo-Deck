//! Orvo: NineDeck pad firmware status and one-click firmware updates.
//! The NineDeck plugin owns the serial port, so the app asks it to do the update
//! and the plugin reports progress back with an "orvoPadStatus" event.

use super::Error;

use tauri::{Emitter, Manager, command};

pub const NINEDECK_PLUGIN: &str = "com.ninedeck.device.sdPlugin";

pub async fn forward_status(payload: serde_json::Value) -> Result<(), anyhow::Error> {
	let app = crate::APP_HANDLE.get().unwrap();
	app.get_webview_window("main").unwrap().emit("pad_status", payload)?;
	Ok(())
}

#[command]
pub async fn request_pad_status() -> Result<(), Error> {
	crate::events::outbound::orvo_send_to_plugin(NINEDECK_PLUGIN, &serde_json::json!({ "event": "orvoPadStatusRequest" })).await?;
	Ok(())
}

#[command]
pub async fn update_pad_firmware(device: String) -> Result<(), Error> {
	crate::events::outbound::orvo_send_to_plugin(NINEDECK_PLUGIN, &serde_json::json!({ "event": "orvoUpdateFirmware", "device": device })).await?;
	Ok(())
}
