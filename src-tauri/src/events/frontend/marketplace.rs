//! Orvo: the Elgato Marketplace inside Orvo Studio.
//!
//! The Marketplace opens in its own window with its own browser profile (so signing out never
//! touches Orvo's settings). Who is signed in is checked here, from that window's own cookies, after
//! each page finishes loading; nothing runs on Elgato's pages while you browse. Only when
//! "Open in Stream Deck" is pressed does a small script (`marketplace_bridge.js`) ask Elgato for the
//! plugin's download link. It talks to Orvo by navigating to `https://orvo-bridge.invalid/...`,
//! which is caught here and cancelled, so Elgato's pages never get access to Orvo itself.
//! If that direct route fails, the Marketplace's own "Open in Stream Deck" link and any plugin
//! file it downloads are caught here too and installed instead.

use super::Error;

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde::Serialize;
use tauri::webview::{DownloadEvent, NewWindowResponse, PageLoadEvent};
use tauri::{AppHandle, Emitter, Manager, Url, WebviewUrl, WebviewWindowBuilder};

const LABEL: &str = "elgato-marketplace";
const HOME: &str = "https://marketplace.elgato.com/stream-deck/plugins";
const BRIDGE_HOST: &str = "orvo-bridge.invalid";
const BRIDGE_JS: &str = include_str!("marketplace_bridge.js");

/// Orvo's fetch add-on (tauri-plugin-cors-fetch) replaces `window.fetch` in every window and routes
/// requests through Orvo, which websites aren't allowed to use, so their requests never finish.
/// This gives the Marketplace window (and its sign-in pop-ups) the browser's own `fetch` back.
const NATIVE_FETCH_JS: &str = r#"(() => {
	const native = window.fetchNative || window.fetch;
	try {
		Object.defineProperty(window, "fetch", { configurable: true, enumerable: true, get() { return native; }, set(_) {} });
	} catch {}
})();"#;

static PENDING: Mutex<Vec<InstallRequest>> = Mutex::new(Vec::new());

static POPUPS: AtomicUsize = AtomicUsize::new(0);
static LAST_ACCOUNT: Mutex<String> = Mutex::new(String::new());

#[derive(Clone, Serialize)]
pub struct InstallRequest {
	name: String,
	url: Option<String>,
	file: Option<String>,
}

#[derive(Clone, Serialize)]
struct Account {
	signed_in: bool,
	name: String,
	email: String,
}

fn data_dir() -> std::path::PathBuf {
	crate::shared::config_dir().join("elgato-marketplace")
}

fn query(url: &Url, key: &str) -> String {
	url.query_pairs().find(|(k, _)| k == key).map(|(_, v)| v.into_owned()).unwrap_or_default()
}

fn bring_main_forward(app: &AppHandle) {
	if let Some(main) = app.get_webview_window("main") {
		let _ = main.unminimize();
		let _ = main.show();
		let _ = main.set_focus();
	}
}

fn request_install(app: &AppHandle, request: InstallRequest) {
	log::info!("Elgato Marketplace: installing {}", request.name);
	let _ = app.emit("elgato_install", request);
	bring_main_forward(app);
}

/// Returns false for navigations Orvo handles itself (they are cancelled).
fn handle_navigation(app: &AppHandle, url: &Url) -> bool {
	if url.host_str() == Some(BRIDGE_HOST) {
		match url.path() {
			"/install" => {
				let link = query(url, "url");
				if link.starts_with("https://") {
					let name = query(url, "name");
					request_install(
						app,
						InstallRequest {
							name: if name.is_empty() { "Elgato plugin".into() } else { name },
							url: Some(link),
							file: None,
						},
					);
				}
			}
			_ => log::warn!("Elgato Marketplace: {}", query(url, "msg")),
		}
		return false;
	}
	if url.scheme() == "streamdeck" {
		// The Marketplace's own hand-off to the Stream Deck app: install from any download link in it.
		log::info!("Elgato Marketplace hand-off: {url}");
		let link = url.query_pairs().map(|(_, v)| v.into_owned()).find(|v| v.starts_with("https://"));
		match link {
			Some(link) => request_install(
				app,
				InstallRequest {
					name: "Elgato plugin".into(),
					url: Some(link),
					file: None,
				},
			),
			None => {
				let _ = app.emit("elgato_notice", url.to_string());
			}
		}
		return false;
	}
	true
}

fn emit_account(app: &AppHandle, account: Account) {
	let key = format!("{}|{}|{}", account.signed_in, account.name, account.email);
	{
		let mut last = LAST_ACCOUNT.lock().unwrap();
		if *last == key {
			return;
		}
		*last = key;
	}
	let _ = app.emit("elgato_account", account);
}

/// Asks Elgato who is signed in, using the Marketplace window's own cookies.
async fn check_account(app: AppHandle, window: tauri::WebviewWindow) {
	let site: Url = "https://marketplace.elgato.com/".parse().unwrap();
	let cookies = match tauri::async_runtime::spawn_blocking(move || window.cookies_for_url(site)).await {
		Ok(Ok(cookies)) => cookies,
		_ => return,
	};
	let header = cookies.iter().map(|c| format!("{}={}", c.name(), c.value())).collect::<Vec<_>>().join("; ");
	let session: serde_json::Value = match reqwest::Client::new()
		.get("https://marketplace.elgato.com/api/auth/session")
		.header("Cookie", header)
		.header("Accept", "application/json")
		.send()
		.await
	{
		Ok(response) => response.json().await.unwrap_or(serde_json::Value::Null),
		Err(error) => {
			log::warn!("Elgato Marketplace: account check failed: {error}");
			return;
		}
	};
	let user = &session["user"];
	let text = |key: &str| user[key].as_str().unwrap_or_default().to_owned();
	emit_account(
		&app,
		Account {
			signed_in: user.is_object(),
			name: if text("name").is_empty() { text("nickname") } else { text("name") },
			email: text("email"),
		},
	);
}

fn is_plugin_file(path: &std::path::Path) -> bool {
	let lower = path.to_string_lossy().to_lowercase();
	lower.ends_with(".streamdeckplugin") || lower.ends_with(".zip")
}

fn build_window(app: &AppHandle, url: Url, visible: bool) -> Result<tauri::WebviewWindow, Error> {
	let nav_app = app.clone();
	let popup_app = app.clone();
	let download_app = app.clone();
	let load_app = app.clone();
	let window = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::External(url))
		.title("Elgato Marketplace · Orvo Studio")
		.inner_size(1200.0, 820.0)
		.min_inner_size(720.0, 520.0)
		.visible(visible)
		.data_directory(data_dir())
		.devtools(true) // right-click → Inspect, to see what a page is doing
		.initialization_script(NATIVE_FETCH_JS)
		.initialization_script(BRIDGE_JS)
		.on_page_load(move |window, payload| {
			if payload.event() == PageLoadEvent::Finished && payload.url().host_str() == Some("marketplace.elgato.com") {
				tauri::async_runtime::spawn(check_account(load_app.clone(), window));
			}
		})
		.on_navigation(move |url| handle_navigation(&nav_app, url))
		// Sign-in pop-ups (Google, Discord, Twitch…) open as small windows that share the session.
		.on_new_window(move |url, features| {
			let label = format!("elgato-popup-{}", POPUPS.fetch_add(1, Ordering::Relaxed));
			let popup_nav = popup_app.clone();
			match WebviewWindowBuilder::new(&popup_app, label, WebviewUrl::External("about:blank".parse().unwrap()))
				.title(url.host_str().unwrap_or("Sign in"))
				.inner_size(520.0, 720.0)
				.window_features(features)
				.initialization_script(NATIVE_FETCH_JS)
				.on_navigation(move |url| handle_navigation(&popup_nav, url))
				.build()
			{
				Ok(window) => NewWindowResponse::Create { window },
				Err(error) => {
					log::error!("Elgato Marketplace pop-up failed: {error}");
					NewWindowResponse::Deny
				}
			}
		})
		.on_download(move |_webview, event| {
			if let DownloadEvent::Finished { path: Some(path), success: true, .. } = event {
				if is_plugin_file(&path) {
					let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Elgato plugin".into());
					request_install(
						&download_app,
						InstallRequest {
							name,
							url: None,
							file: Some(path.to_string_lossy().into_owned()),
						},
					);
				}
			}
			true
		})
		.build()
		.map_err(|e| anyhow::anyhow!(e.to_string()))?;
	Ok(window)
}

/// A plugin file or Marketplace link handed to Orvo by Windows (opening a .streamDeckPlugin file, or
/// "Open in Stream Deck" in your own browser). Returns true if it was one. `running` is false at
/// start-up, when the window isn't listening yet: the request then waits for `orvo_take_pending_installs`.
pub fn handle_handoff(app: &AppHandle, args: &[String], running: bool) -> bool {
	let request = if let Some(file) = args.iter().find(|a| a.to_lowercase().ends_with(".streamdeckplugin")) {
		let path = std::path::Path::new(file);
		let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Elgato plugin".into());
		InstallRequest {
			name,
			url: None,
			file: Some(file.clone()),
		}
	} else if let Some(link) = args.iter().find(|a| a.starts_with("streamdeck://") && !a.starts_with("streamdeck://plugins/message/")) {
		log::info!("Marketplace hand-off from the browser: {link}");
		let Ok(url) = link.parse::<Url>() else { return false };
		match url.query_pairs().map(|(_, v)| v.into_owned()).find(|v| v.starts_with("https://")) {
			Some(download) => InstallRequest {
				name: "Elgato plugin".into(),
				url: Some(download),
				file: None,
			},
			None => {
				let _ = app.emit("elgato_notice", link.clone());
				bring_main_forward(app);
				return true;
			}
		}
	} else {
		return false;
	};
	if running {
		request_install(app, request);
	} else {
		PENDING.lock().unwrap().push(request);
	}
	true
}

/// Plugin files Windows opened Orvo with at start-up, before the window was ready.
#[tauri::command]
pub async fn orvo_take_pending_installs() -> Vec<InstallRequest> {
	std::mem::take(&mut *PENDING.lock().unwrap())
}

/// Opens (or brings forward) the Marketplace window, optionally at a given Marketplace page.
#[tauri::command]
pub async fn orvo_open_marketplace(app: AppHandle, page: Option<String>) -> Result<(), Error> {
	let target: Url = page
		.filter(|p| p.starts_with("https://marketplace.elgato.com/"))
		.unwrap_or_else(|| HOME.to_owned())
		.parse::<Url>()
		.map_err(|e| anyhow::anyhow!(e.to_string()))?;
	if let Some(window) = app.get_webview_window(LABEL) {
		let _ = window.navigate(target);
		let _ = window.unminimize();
		let _ = window.show();
		let _ = window.set_focus();
		return Ok(());
	}
	build_window(&app, target, true)?;
	Ok(())
}

/// Signs out by wiping the Marketplace window's own browser data (Orvo's settings are separate).
#[tauri::command]
pub async fn orvo_marketplace_sign_out(app: AppHandle) -> Result<(), Error> {
	let window = match app.get_webview_window(LABEL) {
		Some(window) => window,
		None => build_window(&app, "about:blank".parse().unwrap(), false)?,
	};
	window.clear_all_browsing_data().map_err(|e| anyhow::anyhow!(e.to_string()))?;
	let _ = window.close();
	emit_account(
		&app,
		Account {
			signed_in: false,
			name: String::new(),
			email: String::new(),
		},
	);
	Ok(())
}
