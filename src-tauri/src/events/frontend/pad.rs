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

// ---------- Slider lock (overlay) ----------
// Locked: one "global" slider action sits on top of every page. Each page's own slider action is kept
// underneath (the stash) and comes back when the slider is unlocked. Saved per device in
// profiles/<device>.orvo-slider.json.

use crate::shared::ActionInstance;
use std::collections::HashMap;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct SliderOverlay {
	locked: bool,
	global: Option<ActionInstance>,
	#[serde(default)]
	stash: HashMap<String, Option<ActionInstance>>,
}

impl Default for SliderOverlay {
	fn default() -> Self {
		Self {
			locked: true,
			global: None,
			stash: HashMap::new(),
		}
	}
}

static OVERLAYS: std::sync::LazyLock<tokio::sync::Mutex<HashMap<String, SliderOverlay>>> = std::sync::LazyLock::new(Default::default);

fn overlay_path(device: &str) -> std::path::PathBuf {
	crate::shared::config_dir().join("profiles").join(format!("{device}.orvo-slider.json"))
}

fn load_overlay(map: &mut HashMap<String, SliderOverlay>, device: &str) -> SliderOverlay {
	map.entry(device.to_owned())
		.or_insert_with(|| std::fs::read(overlay_path(device)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default())
		.clone()
}

fn save_overlay(map: &mut HashMap<String, SliderOverlay>, device: &str, ov: SliderOverlay) {
	if let Ok(b) = serde_json::to_vec_pretty(&ov) {
		let _ = std::fs::write(overlay_path(device), b);
	}
	map.insert(device.to_owned(), ov);
}

fn for_profile(inst: &Option<ActionInstance>, profile: &str) -> Option<ActionInstance> {
	inst.clone().map(|mut i| {
		i.context.profile = profile.to_owned();
		if let Some(children) = &mut i.children {
			for c in children.iter_mut() {
				c.context.profile = profile.to_owned();
			}
		}
		i
	})
}

fn same_instance(a: &Option<ActionInstance>, b: &Option<ActionInstance>) -> bool {
	match (a, b) {
		(Some(a), Some(b)) => a.action.uuid == b.action.uuid && a.settings == b.settings && a.states.len() == b.states.len(),
		(None, None) => true,
		_ => false,
	}
}

/// Put `slot_for(profile)` into every page's slider slot. Pages not stashed yet are stashed first
/// when `stash_first` is set. The page on the pad gets willDisappear / willAppear for what changed.
async fn apply_slider(device: &str, ov: &mut SliderOverlay, unlocking: bool) -> Result<(), anyhow::Error> {
	use crate::store::profiles::acquire_locks_mut;
	let profiles = crate::store::profiles::get_device_profiles(device)?;
	let Some(dev) = crate::shared::DEVICES.get(device).map(|d| d.clone()) else { return Ok(()) };
	let mut gone = vec![];
	let mut came = vec![];
	{
		let mut locks = acquire_locks_mut().await;
		let selected = locks.device_stores.get_selected_profile(device)?;
		for p in profiles {
			let store = locks.profile_stores.get_profile_store_mut(&dev, &p).await?;
			if store.value.sliders.is_empty() {
				continue;
			}
			let want = if unlocking {
				match ov.stash.remove(&p) {
					Some(own) => for_profile(&own, &p),
					None => continue,
				}
			} else {
				ov.stash.entry(p.clone()).or_insert_with(|| store.value.sliders[0].clone());
				for_profile(&ov.global, &p)
			};
			if same_instance(&store.value.sliders[0], &want) {
				continue;
			}
			let old = std::mem::replace(&mut store.value.sliders[0], want.clone());
			store.save()?;
			if p == selected {
				gone.extend(old);
				came.extend(want);
			}
		}
	}
	for i in &gone {
		let _ = crate::events::outbound::will_appear::will_disappear(i, true).await;
	}
	for i in &came {
		let _ = crate::events::outbound::will_appear::will_appear(i).await;
	}
	Ok(())
}

async fn slot_of(device: &str, profile: &str) -> Result<Option<ActionInstance>, anyhow::Error> {
	let locks = crate::store::profiles::acquire_locks().await;
	let ctx = crate::shared::Context {
		device: device.to_owned(),
		profile: profile.to_owned(),
		controller: "Encoder".into(),
		position: 0,
	};
	Ok(crate::store::profiles::get_slot(&ctx, &locks).await?.clone())
}

pub fn slider_locked(device: &str) -> bool {
	device.starts_with("nd-") && OVERLAYS.try_lock().map(|m| m.get(device).map(|o| o.locked).unwrap_or(true)).unwrap_or(true)
}

#[command]
pub async fn orvo_slider_locked(device: String) -> Result<bool, Error> {
	let mut map = OVERLAYS.lock().await;
	Ok(load_overlay(&mut map, &device).locked)
}

/// Lock / unlock the slider. Locking brings back the last global action (or, the first time, uses
/// this page's slider). Unlocking gives every page its own slider action back.
#[command]
pub async fn orvo_slider_lock(device: String, profile: String, locked: bool) -> Result<(), Error> {
	let mut map = OVERLAYS.lock().await;
	let mut ov = load_overlay(&mut map, &device);
	if locked {
		if ov.global.is_none() {
			ov.global = slot_of(&device, &profile).await?;
		}
		ov.locked = true;
		apply_slider(&device, &mut ov, false).await?;
	} else if ov.locked {
		ov.locked = false;
		apply_slider(&device, &mut ov, true).await?;
		ov.stash.clear();
	}
	save_overlay(&mut map, &device, ov);
	Ok(())
}

/// The slider on `profile` changed (action placed, removed or its settings changed). While locked,
/// that becomes the global action for every page.
#[command]
pub async fn orvo_slider_changed(device: String, profile: String) -> Result<(), Error> {
	let mut map = OVERLAYS.lock().await;
	let mut ov = load_overlay(&mut map, &device);
	if !ov.locked {
		return Ok(());
	}
	let now = slot_of(&device, &profile).await?;
	if same_instance(&now, &for_profile(&ov.global, &profile)) && ov.stash.contains_key(&profile) {
		return Ok(());
	}
	ov.global = now;
	apply_slider(&device, &mut ov, false).await?;
	save_overlay(&mut map, &device, ov);
	Ok(())
}

/// A page was opened (maybe a new one): while locked, make sure it shows the global slider.
#[command]
pub async fn orvo_slider_refresh(device: String) -> Result<(), Error> {
	let mut map = OVERLAYS.lock().await;
	let mut ov = load_overlay(&mut map, &device);
	if ov.locked {
		if ov.global.is_none() && ov.stash.is_empty() {
			// first time: the slider on the open page becomes the one on top
			let selected = crate::store::profiles::acquire_locks_mut().await.device_stores.get_selected_profile(&device)?;
			ov.global = slot_of(&device, &selected).await?;
		}
		apply_slider(&device, &mut ov, false).await?;
		save_overlay(&mut map, &device, ov);
	}
	Ok(())
}

/// "Welcome" / "updated" note from startup, shown once by the app's own pop-up.
pub static STARTUP_NOTICE: std::sync::Mutex<Option<(String, String)>> = std::sync::Mutex::new(None);

#[command]
pub fn orvo_take_startup_notice() -> Option<(String, String)> {
	STARTUP_NOTICE.lock().unwrap().take()
}
