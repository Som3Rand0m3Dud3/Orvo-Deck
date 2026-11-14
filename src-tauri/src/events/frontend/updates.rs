//! Orvo Studio updates: list the releases published on GitHub and install any of them
//! (newer or older), so changing versions is one click from the settings page.

use super::Error;

use crate::built_info;

use serde::Serialize;
use tauri::{AppHandle, command};

pub const RELEASES_API: &str = "https://api.github.com/repos/Som3Rand0m3Dud3/Orvo-Deck/releases";
const USER_AGENT: &str = "OrvoStudio";

#[derive(Serialize, Clone)]
pub struct Release {
	pub tag: String,
	pub version: String,
	pub name: String,
	pub notes: String,
	pub date: String,
	pub page: String,
	/// Download link of the installer for this computer, if the release has one.
	pub installer: Option<String>,
	pub newer: bool,
	pub current: bool,
	/// A test build (GitHub "pre-release"), shown only when test versions are allowed.
	pub prerelease: bool,
}

fn installer_for_this_os(assets: &[serde_json::Value]) -> Option<String> {
	let names_urls = assets
		.iter()
		.filter_map(|a| Some((a.get("name")?.as_str()?.to_lowercase(), a.get("browser_download_url")?.as_str()?.to_owned())));
	let pick = |ok: &dyn Fn(&str) -> bool| names_urls.clone().find(|(n, _)| ok(n)).map(|(_, u)| u);
	if cfg!(windows) {
		pick(&|n| n.ends_with("-setup.exe")).or_else(|| pick(&|n| n.ends_with(".msi")))
	} else if cfg!(target_os = "macos") {
		pick(&|n| n.ends_with(".dmg"))
	} else {
		pick(&|n| n.ends_with(".appimage")).or_else(|| pick(&|n| n.ends_with(".deb")))
	}
}

/// Test versions are included when asked for, which by default is while the app itself is a 0.x test version.
pub fn test_versions_default() -> bool {
	semver::Version::parse(built_info::PKG_VERSION).map(|v| v.major == 0).unwrap_or(false)
}

pub async fn fetch_releases(include_prerelease: bool) -> Result<Vec<Release>, anyhow::Error> {
	let res = reqwest::Client::new()
		.get(format!("{RELEASES_API}?per_page=50"))
		.header("Accept", "application/vnd.github+json")
		.header("User-Agent", USER_AGENT)
		.send()
		.await?
		.error_for_status()?
		.json::<Vec<serde_json::Value>>()
		.await?;

	let current = semver::Version::parse(built_info::PKG_VERSION)?;
	let mut releases = vec![];
	for r in res {
		if r.get("draft").and_then(|v| v.as_bool()).unwrap_or(false) {
			continue;
		}
		let prerelease = r.get("prerelease").and_then(|v| v.as_bool()).unwrap_or(false);
		if prerelease && !include_prerelease {
			continue;
		}
		let Some(tag) = r.get("tag_name").and_then(|v| v.as_str()) else { continue };
		let Ok(version) = semver::Version::parse(tag.trim_start_matches('v')) else { continue };
		let text = |k: &str| r.get(k).and_then(|v| v.as_str()).unwrap_or("").to_owned();
		let assets = r.get("assets").and_then(|v| v.as_array()).cloned().unwrap_or_default();
		releases.push(Release {
			tag: tag.to_owned(),
			version: version.to_string(),
			name: text("name"),
			notes: text("body").trim().to_owned(),
			date: text("published_at").chars().take(10).collect(),
			page: text("html_url"),
			installer: installer_for_this_os(&assets),
			newer: version > current,
			current: version == current,
			prerelease,
		});
	}
	releases.sort_by(|a, b| semver::Version::parse(&b.version).unwrap().cmp(&semver::Version::parse(&a.version).unwrap()));
	Ok(releases)
}

#[command]
pub async fn list_releases(include_prerelease: Option<bool>) -> Result<Vec<Release>, Error> {
	Ok(fetch_releases(include_prerelease.unwrap_or_else(test_versions_default)).await?)
}

/// Downloads the installer of the chosen release, starts it, and closes Orvo Studio so it can be replaced.
#[command]
pub async fn install_release(app: AppHandle, tag: String) -> Result<(), Error> {
	install(&app, &tag).await.map_err(Into::into)
}

pub async fn install(app: &AppHandle, tag: &str) -> Result<(), anyhow::Error> {
	let releases = fetch_releases(true).await?;
	let Some(release) = releases.into_iter().find(|r| r.tag == tag) else {
		return Err(anyhow::anyhow!("Version {tag} was not found"));
	};

	// No installer for this computer (or not Windows): open the release page instead.
	let Some(url) = release.installer.filter(|_| cfg!(windows)) else {
		open::that_detached(&release.page)?;
		return Ok(());
	};

	let bytes = reqwest::Client::new().get(&url).header("User-Agent", USER_AGENT).send().await?.error_for_status()?.bytes().await?;
	let file_name = url.rsplit('/').next().unwrap_or("OrvoStudio-setup.exe").to_owned();
	let installer = std::env::temp_dir().join(file_name);
	tokio::fs::write(&installer, &bytes).await?;

	// Remember to update the pad too once the new version starts.
	let _ = std::fs::write(crate::shared::config_dir().join("orvo-update-pending"), tag);

	// One click: a hidden helper waits for Orvo Studio to close, runs the installer in passive
	// mode (progress bar only, no questions), deletes the installer and starts the new version.
	let exe = std::env::current_exe()?;
	let quote = |p: &std::path::Path| p.to_string_lossy().replace('\'', "''");
	let script = format!(
		"Start-Sleep -Milliseconds 800; Start-Process -FilePath '{installer}' -ArgumentList '/P' -Wait; Remove-Item -LiteralPath '{installer}' -Force -ErrorAction SilentlyContinue; Start-Process -FilePath '{exe}'",
		installer = quote(&installer),
		exe = quote(&exe),
	);
	let mut command = std::process::Command::new("powershell");
	command.args(["-NoProfile", "-WindowStyle", "Hidden", "-ExecutionPolicy", "Bypass", "-Command", &script]);
	#[cfg(windows)]
	{
		use std::os::windows::process::CommandExt;
		command.creation_flags(0x08000000); // CREATE_NO_WINDOW
	}
	command.spawn()?;
	app.exit(0);
	Ok(())
}
