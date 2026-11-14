//! Orvo: the Elgato Marketplace inside Orvo Studio's Elgato tab.
//!
//! The Marketplace is a second web view placed inside the main window, over the Elgato tab's area,
//! with its own browser profile (so signing out never touches Orvo's settings and the sign-in is
//! kept between sessions). Nothing is injected into Elgato's pages except giving them the
//! browser's own `fetch` back (see `NATIVE_FETCH_JS`).
//!
//! Installing: the Marketplace's "Open in Stream Deck" hands over `streamdeck://plugins/install/<id>`.
//! Orvo asks Elgato for that item's download from inside the Marketplace page (exactly as the page
//! itself would, with its own sign-in), reads the answer back, then installs it.
//! "Download" is caught too: any downloaded file that is a plugin gets installed.

use super::Error;

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde::Serialize;
use tauri::webview::{DownloadEvent, NewWindowResponse, PageLoadEvent};
use tauri::{AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Url, Webview, WebviewBuilder, WebviewUrl, WebviewWindowBuilder};

const LABEL: &str = "elgato-marketplace";
const HOME: &str = "https://marketplace.elgato.com/stream-deck/plugins";

/// Orvo's fetch add-on (tauri-plugin-cors-fetch) replaces `window.fetch` in every web view and routes
/// requests through Orvo, which websites aren't allowed to use, so their requests never finish.
/// This gives the Marketplace (and its sign-in pop-ups) the browser's own `fetch` back.
const NATIVE_FETCH_JS: &str = r#"(() => {
	const native = window.fetchNative || window.fetch;
	try {
		Object.defineProperty(window, "fetch", { configurable: true, enumerable: true, get() { return native; }, set(_) {} });
	} catch {}
})();"#;

static POPUPS: AtomicUsize = AtomicUsize::new(0);
static LAST_ACCOUNT: Mutex<String> = Mutex::new(String::new());
static PENDING: Mutex<Vec<InstallRequest>> = Mutex::new(Vec::new());
static JOBS: AtomicUsize = AtomicUsize::new(0);

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

fn notice(app: &AppHandle, text: String) {
	log::warn!("Elgato Marketplace: {text}");
	let _ = app.emit("elgato_notice", text);
	bring_main_forward(app);
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

/// The Marketplace web view, created (hidden) if it doesn't exist yet.
fn marketplace(app: &AppHandle) -> Result<Webview<tauri::Wry>, Error> {
	if let Some(view) = app.get_webview(LABEL) {
		return Ok(view);
	}
	let window = app.get_window("main").ok_or_else(|| anyhow::anyhow!("main window not found"))?;
	let nav_app = app.clone();
	let popup_app = app.clone();
	let download_app = app.clone();
	let load_app = app.clone();
	let builder = WebviewBuilder::new(LABEL, WebviewUrl::External(HOME.parse().unwrap()))
		.data_directory(data_dir())
		.devtools(true) // right-click → Inspect, to see what a page is doing
		.initialization_script(NATIVE_FETCH_JS)
		.on_navigation(move |url| handle_navigation(&nav_app, url))
		.on_page_load(move |view, payload| {
			if payload.event() == PageLoadEvent::Finished && payload.url().host_str() == Some("marketplace.elgato.com") {
				tauri::async_runtime::spawn(check_account(load_app.clone(), view));
			}
		})
		// Sign-in pop-ups (Discord, Twitch…) open as small windows that share the session.
		.on_new_window(move |url, features| {
			let label = format!("elgato-popup-{}", POPUPS.fetch_add(1, Ordering::Relaxed));
			let popup_nav = popup_app.clone();
			let popup_download = popup_app.clone();
			match WebviewWindowBuilder::new(&popup_app, label, WebviewUrl::External("about:blank".parse().unwrap()))
				.title(url.host_str().unwrap_or("Sign in"))
				.inner_size(520.0, 720.0)
				.window_features(features)
				.initialization_script(NATIVE_FETCH_JS)
				.on_navigation(move |url| handle_navigation(&popup_nav, url))
				.on_download(move |_, event| handle_download(&popup_download, event))
				.build()
			{
				Ok(window) => NewWindowResponse::Create { window },
				Err(error) => {
					log::error!("Elgato Marketplace pop-up failed: {error}");
					NewWindowResponse::Deny
				}
			}
		})
		.on_download(move |_, event| handle_download(&download_app, event));
	let view = window
		.add_child(builder, LogicalPosition::new(0.0, 0.0), LogicalSize::new(10.0, 10.0))
		.map_err(|e| anyhow::anyhow!(e.to_string()))?;
	let _ = view.hide();
	Ok(view)
}

/// Returns false for navigations Orvo handles itself (they are cancelled).
fn handle_navigation(app: &AppHandle, url: &Url) -> bool {
	if url.scheme() == "streamdeck" {
		handle_link(app, url.as_str());
		return false;
	}
	true
}

/// `streamdeck://plugins/install/<item id>` from the Marketplace (in Orvo, or in your own browser).
fn handle_link(app: &AppHandle, link: &str) {
	log::info!("Elgato Marketplace hand-off: {link}");
	let Ok(url) = link.parse::<Url>() else { return };
	if let Some(download) = url.query_pairs().map(|(_, v)| v.into_owned()).find(|v| v.starts_with("https://")) {
		request_install(
			app,
			InstallRequest {
				name: "Elgato plugin".into(),
				url: Some(download),
				file: None,
			},
		);
		return;
	}
	let segments: Vec<String> = url.path_segments().map(|s| s.filter(|s| !s.is_empty()).map(str::to_owned).collect()).unwrap_or_default();
	if url.host_str() == Some("plugins") && segments.first().map(String::as_str) == Some("install") && segments.len() >= 2 {
		let item = segments[1].clone();
		tauri::async_runtime::spawn(install_item(app.clone(), item));
	} else {
		notice(app, format!("Orvo received a Marketplace link it doesn't know yet.\n\nLink received: {link}"));
	}
}

/// Runs `body` (the inside of an async JS function) in the Marketplace page and returns what it
/// returns, as the page itself would see it (its own sign-in, its own access to Elgato).
async fn run_in_page(view: &Webview<tauri::Wry>, body: &str, timeout: std::time::Duration) -> Result<serde_json::Value, String> {
	let started = std::time::Instant::now();

	// The result of a script comes back JSON-encoded; a string result is encoded twice.
	fn decode(raw: String) -> serde_json::Value {
		match serde_json::from_str::<serde_json::Value>(&raw) {
			Ok(serde_json::Value::String(inner)) => serde_json::from_str(&inner).unwrap_or(serde_json::Value::String(inner)),
			Ok(value) => value,
			Err(_) => serde_json::Value::Null,
		}
	}
	async fn ask(view: &Webview<tauri::Wry>, js: String) -> Option<serde_json::Value> {
		let (tx, rx) = std::sync::mpsc::channel::<String>();
		view.eval_with_callback(js, move |result| {
			let _ = tx.send(result);
		})
		.ok()?;
		let raw = tauri::async_runtime::spawn_blocking(move || rx.recv_timeout(std::time::Duration::from_secs(3)).ok())
			.await
			.ok()
			.flatten()?;
		Some(decode(raw))
	}

	// wait for a loaded Marketplace page
	loop {
		let ready = ask(view, "JSON.stringify(location.hostname === 'marketplace.elgato.com' && document.readyState === 'complete')".into()).await;
		if ready == Some(serde_json::Value::Bool(true)) {
			break;
		}
		if started.elapsed() > timeout {
			return Err("The Marketplace page didn't finish loading.".into());
		}
		tokio::time::sleep(std::time::Duration::from_millis(400)).await;
	}

	let slot = format!("__orvo_job_{}", JOBS.fetch_add(1, Ordering::Relaxed));
	let script = format!(
		"(async () => {{ try {{ const value = await (async () => {{ {body} }})(); window.{slot} = {{ ok: true, value }}; }} catch (e) {{ window.{slot} = {{ ok: false, error: String(e) }}; }} }})(); null"
	);
	if view.eval(script).is_err() {
		return Err("Couldn't run inside the Marketplace page.".into());
	}
	loop {
		if let Some(result) = ask(view, format!("JSON.stringify(window.{slot} ?? null)")).await
			&& !result.is_null()
		{
			let _ = view.eval(format!("delete window.{slot}"));
			return if result["ok"] == serde_json::Value::Bool(true) {
				Ok(result["value"].clone())
			} else {
				Err(result["error"].as_str().unwrap_or("unknown error").to_owned())
			};
		}
		if started.elapsed() > timeout {
			return Err("The Marketplace page didn't answer in time.".into());
		}
		tokio::time::sleep(std::time::Duration::from_millis(300)).await;
	}
}

const SESSION_JS: &str = "const r = await fetch('/api/auth/session', { credentials: 'include' }); return r.ok ? await r.json() : null;";

/// The signed-in session, read by the Marketplace page itself (`None` when signed out).
async fn session(app: &AppHandle) -> Option<serde_json::Value> {
	let view = marketplace(app).ok()?;
	let value = run_in_page(&view, SESSION_JS, std::time::Duration::from_secs(20)).await.ok()?;
	value["user"].is_object().then_some(value)
}

/// Asks Elgato for an item's download from inside the Marketplace page, then installs it.
async fn install_item(app: AppHandle, item: String) {
	if item.is_empty() || !item.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
		return notice(&app, format!("Orvo received a Marketplace item it can't read: {item}"));
	}
	let view = match marketplace(&app) {
		Ok(view) => view,
		Err(error) => return notice(&app, format!("Couldn't open the Marketplace: {error:?}")),
	};
	let script = format!(
		"const s = await (await fetch('/api/auth/session', {{ credentials: 'include' }})).json();
		if (!s || !s.user) return {{ signedOut: true }};
		const r = await fetch('https://mp-gateway.elgato.com/items/{item}/purchase-link', {{
			headers: {{ Authorization: 'Bearer ' + s.accessToken, 'Content-Type': 'application/json' }}
		}});
		return {{ status: r.status, body: await r.text(), hasToken: !!s.accessToken }};"
	);
	let result = match run_in_page(&view, &script, std::time::Duration::from_secs(30)).await {
		Ok(result) => result,
		Err(error) => return notice(&app, format!("Couldn't get this plugin from Elgato: {error}")),
	};
	if result["signedOut"] == serde_json::Value::Bool(true) {
		return notice(&app, "Sign in to the Elgato Marketplace in Orvo's Elgato tab first, then press Open in Stream Deck again.".into());
	}
	let status = result["status"].as_u64().unwrap_or(0);
	let body = result["body"].as_str().unwrap_or_default();
	let json: serde_json::Value = serde_json::from_str(body).unwrap_or(serde_json::Value::Null);
	match json["direct_link"].as_str() {
		Some(link) if (200..300).contains(&status) => {
			let name = ["name", "product_name", "title"].iter().find_map(|k| json[*k].as_str()).unwrap_or("Elgato plugin").to_owned();
			request_install(
				&app,
				InstallRequest {
					name,
					url: Some(link.to_owned()),
					file: None,
				},
			);
		}
		_ => {
			let short: String = body.chars().take(300).collect();
			let token = if result["hasToken"] == serde_json::Value::Bool(false) {
				" (no download key in the sign-in)"
			} else {
				""
			};
			notice(
				&app,
				format!("Elgato didn't give a download for this plugin (item {item}){token}.\n\nElgato said: {status} {short}\n\nUse Download on the plugin's page instead."),
			);
		}
	}
}

/// Who is signed in, checked after each Marketplace page loads.
async fn check_account(app: AppHandle, _view: Webview<tauri::Wry>) {
	let user = session(&app).await.map(|s| s["user"].clone());
	let text = |key: &str| user.as_ref().and_then(|u| u[key].as_str()).unwrap_or_default().to_owned();
	emit_account(
		&app,
		Account {
			signed_in: user.is_some(),
			name: if text("name").is_empty() { text("nickname") } else { text("name") },
			email: text("email"),
		},
	);
}

/// A finished download that is a plugin (a zip archive, whatever its name) gets installed.
fn handle_download(app: &AppHandle, event: DownloadEvent<'_>) -> bool {
	if let DownloadEvent::Finished { path: Some(path), success: true, .. } = event {
		let is_zip = std::fs::read(&path).map(|b| b.starts_with(b"PK")).unwrap_or(false);
		log::info!("Elgato Marketplace download: {} (plugin: {is_zip})", path.display());
		if is_zip {
			let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Elgato plugin".into());
			request_install(
				app,
				InstallRequest {
					name,
					url: None,
					file: Some(path.to_string_lossy().into_owned()),
				},
			);
		}
	}
	true
}

/// A plugin file or Marketplace link handed to Orvo by Windows (opening a .streamDeckPlugin file, or
/// "Open in Stream Deck" in your own browser). Returns true if it was one. `running` is false at
/// start-up, when the window isn't listening yet: files then wait for `orvo_take_pending_installs`.
pub fn handle_handoff(app: &AppHandle, args: &[String], running: bool) -> bool {
	if let Some(file) = args.iter().find(|a| a.to_lowercase().ends_with(".streamdeckplugin")) {
		let path = std::path::Path::new(file);
		let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Elgato plugin".into());
		let request = InstallRequest {
			name,
			url: None,
			file: Some(file.clone()),
		};
		if running {
			request_install(app, request);
		} else {
			PENDING.lock().unwrap().push(request);
		}
		return true;
	}
	if let Some(link) = args.iter().find(|a| a.starts_with("streamdeck://") && !a.starts_with("streamdeck://plugins/message/")) {
		if running {
			handle_link(app, link);
		} else {
			let app = app.clone();
			let link = link.clone();
			// give the window a moment to start listening
			tauri::async_runtime::spawn(async move {
				tokio::time::sleep(std::time::Duration::from_secs(3)).await;
				handle_link(&app, &link);
			});
		}
		return true;
	}
	false
}

/// Plugin files Windows opened Orvo with at start-up, before the window was ready.
#[tauri::command]
pub async fn orvo_take_pending_installs() -> Vec<InstallRequest> {
	std::mem::take(&mut *PENDING.lock().unwrap())
}

/// Shows the Marketplace over the Elgato tab's area (window coordinates, in CSS pixels).
#[tauri::command]
pub async fn orvo_marketplace_show(app: AppHandle, x: f64, y: f64, width: f64, height: f64) -> Result<(), Error> {
	let view = marketplace(&app)?;
	let _ = view.set_position(LogicalPosition::new(x, y));
	let _ = view.set_size(LogicalSize::new(width.max(10.0), height.max(10.0)));
	let _ = view.show();
	Ok(())
}

#[tauri::command]
pub async fn orvo_marketplace_hide(app: AppHandle) -> Result<(), Error> {
	if let Some(view) = app.get_webview(LABEL) {
		let _ = view.hide();
	}
	Ok(())
}

/// Goes back to the Marketplace's plugin list.
#[tauri::command]
pub async fn orvo_marketplace_home(app: AppHandle) -> Result<(), Error> {
	let view = marketplace(&app)?;
	let _ = view.navigate(HOME.parse().unwrap());
	Ok(())
}

/// The Marketplace's back button.
#[tauri::command]
pub async fn orvo_marketplace_back(app: AppHandle) -> Result<(), Error> {
	if let Some(view) = app.get_webview(LABEL) {
		let _ = view.eval("history.back()");
	}
	Ok(())
}

/// Signs out by wiping the Marketplace's own browser data (Orvo's settings are separate).
#[tauri::command]
pub async fn orvo_marketplace_sign_out(app: AppHandle) -> Result<(), Error> {
	let view = marketplace(&app)?;
	view.clear_all_browsing_data().map_err(|e| anyhow::anyhow!(e.to_string()))?;
	let _ = view.navigate(HOME.parse().unwrap());
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
