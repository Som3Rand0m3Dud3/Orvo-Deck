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

pub async fn forward_slider(payload: serde_json::Value) -> Result<(), anyhow::Error> {
	let app = crate::APP_HANDLE.get().unwrap();
	app.get_webview_window("main").unwrap().emit("pad_slider", payload)?;
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

/// A page (profile) of a device without selecting it, so the pad's pages can be drawn ahead of time.
#[command]
pub async fn orvo_get_profile(device: String, id: String) -> Result<crate::shared::Profile, Error> {
	let mut locks = crate::store::profiles::acquire_locks_mut().await;
	let Some(info) = crate::shared::DEVICES.get(&device).map(|d| d.clone()) else {
		return Err(anyhow::anyhow!("device {device} not found").into());
	};
	let store = locks.profile_stores.get_profile_store_mut(&info, &id).await?;
	Ok(store.value.clone())
}

/// Hands the NineDeck plugin the finished pictures of one page, so it can show that page instantly.
#[command]
pub async fn orvo_page_images(device: String, profile: String, images: Vec<Option<String>>) -> Result<(), Error> {
	crate::events::outbound::orvo_send_to_plugin(
		NINEDECK_PLUGIN,
		&serde_json::json!({ "event": "orvoPageImages", "device": device, "profile": profile, "images": images }),
	)
	.await?;
	Ok(())
}

/// Slide speed for page changes on the pad (ms, 0 = off), chosen in Settings.
#[command]
pub async fn orvo_pad_settings(slide_ms: Option<u32>, taper: Option<u8>, recalibrate: Option<bool>, flip: Option<bool>) -> Result<(), Error> {
	let mut msg = serde_json::json!({ "event": "orvoPadSettings" });
	if let Some(f) = flip {
		msg["flip"] = (f as u32).into();
	}
	if let Some(ms) = slide_ms {
		msg["slideMs"] = ms.min(1000).into();
	}
	if let Some(t) = taper {
		msg["taper"] = (t.min(1) as u32).into();
	}
	if recalibrate.unwrap_or(false) {
		msg["recalibrate"] = true.into();
	}
	crate::events::outbound::orvo_send_to_plugin(NINEDECK_PLUGIN, &msg).await?;
	Ok(())
}

/// True once after Orvo Studio updated itself, so the app can update the pad straight after.
#[command]
pub fn orvo_take_update_marker() -> bool {
	let marker = crate::shared::config_dir().join("orvo-update-pending");
	let present = marker.exists();
	let _ = std::fs::remove_file(marker);
	present
}

/// Slider lock: copy the slider's action (and its settings) from `profile` to every other page of the
/// device, so the slider does the same thing on all pages. An empty slider empties it everywhere.
#[command]
pub async fn orvo_sync_slider(device: String, profile: String) -> Result<(), Error> {
	use crate::store::profiles::{acquire_locks_mut, get_slot_mut};
	let profiles = crate::store::profiles::get_device_profiles(&device)?;
	let mut locks = acquire_locks_mut().await;
	let src_ctx = crate::shared::Context { device: device.clone(), profile: profile.clone(), controller: "Encoder".into(), position: 0 };
	let source = get_slot_mut(&src_ctx, &mut locks).await?.clone();
	let Some(dev) = crate::shared::DEVICES.get(&device).map(|d| d.clone()) else { return Ok(()) };
	for other in profiles.into_iter().filter(|p| *p != profile) {
		let copy = source.clone().map(|mut inst| {
			inst.context.profile = other.clone();
			if let Some(children) = &mut inst.children {
				for c in children.iter_mut() {
					c.context.profile = other.clone();
				}
			}
			inst
		});
		let store = locks.profile_stores.get_profile_store_mut(&dev, &other).await?;
		if store.value.sliders.is_empty() {
			continue;
		}
		let same = match (&store.value.sliders[0], &copy) {
			(Some(a), Some(b)) => a.action.uuid == b.action.uuid && a.settings == b.settings,
			(None, None) => true,
			_ => false,
		};
		if !same {
			store.value.sliders[0] = copy;
			store.save()?;
		}
	}
	Ok(())
}

/// Devices whose slider is locked (same action on every page). Set by the app; locked by default.
static UNLOCKED_SLIDERS: std::sync::LazyLock<std::sync::Mutex<std::collections::HashSet<String>>> = std::sync::LazyLock::new(Default::default);

pub fn slider_locked(device: &str) -> bool {
	device.starts_with("nd-") && !UNLOCKED_SLIDERS.lock().unwrap().contains(device)
}

#[command]
pub async fn orvo_slider_lock(device: String, profile: String, locked: bool) -> Result<(), Error> {
	if locked {
		UNLOCKED_SLIDERS.lock().unwrap().remove(&device);
		orvo_sync_slider(device, profile).await
	} else {
		UNLOCKED_SLIDERS.lock().unwrap().insert(device);
		Ok(())
	}
}
