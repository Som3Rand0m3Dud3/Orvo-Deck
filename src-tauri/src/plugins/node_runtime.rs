//! Finds a Node.js runtime for JavaScript plugins.
//!
//! Order: Orvo's own copy (downloaded once into the app data folder), then a
//! Node.js 20+ already installed on the PC. On Windows, if neither exists,
//! Orvo downloads the official Node.js LTS build from nodejs.org, checks its
//! SHA-256 against the published list and keeps just node.exe.

use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, anyhow};
use tauri::{Emitter, Manager};
use tokio::sync::Mutex;

/// Node.js release line Orvo downloads.
const NODE_LINE: &str = "latest-v22.x";
const MIN_MAJOR: u32 = 20;

static LOCK: Mutex<()> = Mutex::const_new(());

/// How to launch Node: the program plus any arguments that come before the script.
pub struct NodeCmd {
	pub program: String,
	pub pre_args: Vec<String>,
}

fn managed_dir() -> PathBuf {
	let app = crate::APP_HANDLE.get().unwrap();
	app.path().app_data_dir().unwrap().join("runtime").join("node")
}

fn managed_exe() -> PathBuf {
	managed_dir().join(if cfg!(windows) { "node.exe" } else { "node" })
}

/// Runs `<program> [pre_args] --version` and returns the major version.
fn major_version(program: &str, pre_args: &[String]) -> Option<u32> {
	let mut cmd = Command::new(program);
	cmd.args(pre_args).arg("--version");
	#[cfg(windows)]
	{
		use std::os::windows::process::CommandExt;
		cmd.creation_flags(0x08000000);
	}
	let out = cmd.output().ok()?;
	if !out.status.success() {
		return None;
	}
	let text = String::from_utf8_lossy(&out.stdout);
	text.trim().trim_start_matches('v').split('.').next()?.parse().ok()
}

fn notice(message: &str) {
	if let Some(app) = crate::APP_HANDLE.get() {
		let _ = app.emit("elgato_notice", message);
	}
}

pub async fn resolve() -> anyhow::Result<NodeCmd> {
	let _guard = LOCK.lock().await;

	let managed = managed_exe();
	if managed.exists() && major_version(managed.to_str().unwrap_or_default(), &[]).is_some_and(|m| m >= MIN_MAJOR) {
		return Ok(NodeCmd {
			program: managed.to_string_lossy().into_owned(),
			pre_args: vec![],
		});
	}

	let (program, pre_args) = if crate::shared::is_flatpak() {
		("flatpak-spawn".to_owned(), vec!["--host".to_owned(), "node".to_owned()])
	} else {
		("node".to_owned(), vec![])
	};
	let system = major_version(&program, &pre_args);
	if system.is_some_and(|m| m >= MIN_MAJOR) {
		return Ok(NodeCmd { program, pre_args });
	}

	if cfg!(windows) {
		notice("Setting up Node.js for JavaScript plugins (one-time download, about 30 MB)…");
		match download().await {
			Ok(path) => {
				notice("Node.js is ready. JavaScript plugins will now start.");
				return Ok(NodeCmd {
					program: path.to_string_lossy().into_owned(),
					pre_args: vec![],
				});
			}
			Err(e) => {
				log::error!("Failed to download Node.js: {e:#}");
				return Err(anyhow!(
					"This plugin needs Node.js, and Orvo couldn't download it ({e}). Check your internet connection and restart Orvo, or install Node.js LTS from nodejs.org."
				));
			}
		}
	}

	Err(match system {
		Some(m) => anyhow!("This plugin needs Node.js {MIN_MAJOR} or newer, but this PC has Node.js {m}. Update it from nodejs.org, then restart Orvo."),
		None => anyhow!("This plugin needs Node.js {MIN_MAJOR} or newer. Install Node.js LTS from nodejs.org, then restart Orvo."),
	})
}

async fn download() -> anyhow::Result<PathBuf> {
	let arch = match std::env::consts::ARCH {
		"x86_64" => "x64",
		"aarch64" => "arm64",
		"x86" => "x86",
		other => return Err(anyhow!("unsupported CPU type {other}")),
	};
	let base = format!("https://nodejs.org/dist/{NODE_LINE}/");
	let client = reqwest::Client::builder().user_agent("Orvo Studio").build()?;

	let sums = client.get(format!("{base}SHASUMS256.txt")).send().await?.error_for_status()?.text().await?;
	let suffix = format!("-win-{arch}.zip");
	let (hash, file) = sums
		.lines()
		.filter_map(|l| l.split_once("  "))
		.find(|(_, name)| name.starts_with("node-v") && name.ends_with(&suffix))
		.context("Node.js build for this PC not found")?;
	let folder = file.trim_end_matches(".zip").to_owned();

	let bytes = client.get(format!("{base}{file}")).send().await?.error_for_status()?.bytes().await?;

	use sha2::{Digest, Sha256};
	let actual = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect::<String>();
	if !actual.eq_ignore_ascii_case(hash.trim()) {
		return Err(anyhow!("download was corrupted (checksum mismatch)"));
	}

	let dir = managed_dir();
	let target = managed_exe();
	tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
		let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))?;
		let mut entry = archive.by_name(&format!("{folder}/node.exe")).context("node.exe missing from download")?;
		std::fs::create_dir_all(&dir)?;
		let tmp = dir.join("node.exe.part");
		let mut out = std::fs::File::create(&tmp)?;
		std::io::copy(&mut entry, &mut out)?;
		drop(out);
		std::fs::rename(&tmp, &target)?;
		Ok(())
	})
	.await??;

	Ok(managed_exe())
}
