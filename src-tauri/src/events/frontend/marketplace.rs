//! Orvo: plugin files and links that Windows hands to Orvo Studio.
//!
//! Opening a `.streamDeckPlugin` file (double-click, or "Open" in a browser's downloads) installs
//! it into Orvo. Elgato Marketplace "Open in Stream Deck" links (`streamdeck://plugins/install/…`)
//! can't be used: Marketplace builds are encrypted for Elgato's own app, so Orvo explains that.

use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

static PENDING: Mutex<Vec<InstallRequest>> = Mutex::new(Vec::new());

#[derive(Clone, Serialize)]
pub struct InstallRequest {
	name: String,
	url: Option<String>,
	file: Option<String>,
}

fn bring_main_forward(app: &AppHandle) {
	if let Some(main) = app.get_webview_window("main") {
		let _ = main.unminimize();
		let _ = main.show();
		let _ = main.set_focus();
	}
}

/// Returns true if `args` held a plugin file or a Marketplace link. `running` is false at start-up,
/// when the window isn't listening yet: files then wait for `orvo_take_pending_installs`.
pub fn handle_handoff(app: &AppHandle, args: &[String], running: bool) -> bool {
	if let Some(file) = args.iter().find(|a| a.to_lowercase().ends_with(".streamdeckplugin")) {
		let path = std::path::Path::new(file);
		let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Plugin".into());
		let request = InstallRequest {
			name,
			url: None,
			file: Some(file.clone()),
		};
		if running {
			let _ = app.emit("elgato_install", request);
			bring_main_forward(app);
		} else {
			PENDING.lock().unwrap().push(request);
		}
		return true;
	}
	if let Some(link) = args.iter().find(|a| a.starts_with("streamdeck://plugins/install/")) {
		log::info!("Elgato Marketplace link received: {link}");
		let app = app.clone();
		let delay = if running { 0 } else { 3 };
		tauri::async_runtime::spawn(async move {
			tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
			let _ = app.emit(
				"elgato_notice",
				"Plugins from the Elgato Marketplace are locked to Elgato's own app, so Orvo can't install them.\n\nFind the same plugin in Orvo's plugin store, or download it from its developer's page and use \"Install from file\".",
			);
			bring_main_forward(&app);
		});
		return true;
	}
	false
}

/// Plugin files Windows opened Orvo with at start-up, before the window was ready.
#[tauri::command]
pub async fn orvo_take_pending_installs() -> Vec<InstallRequest> {
	std::mem::take(&mut *PENDING.lock().unwrap())
}
