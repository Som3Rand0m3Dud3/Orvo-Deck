//! Find sounds on Myinstants (myinstants.com) and add them to the soundboard.
//! Myinstants has no official API: its pages are read like a browser would.

use serde::Serialize;
use tauri::command;

use crate::events::frontend::Error;

const BASE: &str = "https://www.myinstants.com";
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";
const MAX_BYTES: usize = 15 * 1024 * 1024;

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct OnlineSound {
	pub id: String,
	pub title: String,
	pub mp3: String,
}

#[derive(Serialize)]
pub struct OnlinePage {
	sounds: Vec<OnlineSound>,
	more: bool,
}

fn client() -> reqwest::Result<reqwest::Client> {
	reqwest::Client::builder().user_agent(UA).timeout(std::time::Duration::from_secs(20)).build()
}

fn decode_entities(s: &str) -> String {
	let mut out = s
		.replace("&amp;", "&")
		.replace("&quot;", "\"")
		.replace("&#39;", "'")
		.replace("&#x27;", "'")
		.replace("&lt;", "<")
		.replace("&gt;", ">")
		.replace("&nbsp;", " ");
	// numeric entities like &#233;
	while let Some(i) = out.find("&#") {
		let Some(j) = out[i..].find(';').map(|j| i + j) else { break };
		let code = &out[i + 2..j];
		let ch = if let Some(hex) = code.strip_prefix('x') {
			u32::from_str_radix(hex, 16).ok()
		} else {
			code.parse().ok()
		}
		.and_then(char::from_u32);
		match ch {
			Some(c) => out.replace_range(i..=j, &c.to_string()),
			None => break,
		}
	}
	out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Pulls the sound buttons out of a Myinstants list page.
pub fn parse(html: &str) -> Vec<OnlineSound> {
	let mut out = Vec::new();
	let mut rest = html;
	while let Some(p) = rest.find("play('") {
		let after = &rest[p + 6..];
		let Some(end) = after.find('\'') else { break };
		let mp3_rel = &after[..end];
		let tail = &after[end..];
		// the title link comes right after the play button
		let next_play = tail.find("play('").unwrap_or(tail.len());
		let block = &tail[..next_play];
		rest = &tail[next_play..];
		let Some(link) = block.find("instant-link") else { continue };
		let Some(a_start) = block[..link].rfind("<a") else { continue };
		let Some(tag_end) = block[link..].find('>').map(|x| link + x) else { continue };
		let tag = &block[a_start..tag_end];
		let href = tag.split("href=\"").nth(1).and_then(|h| h.split('"').next()).unwrap_or("");
		let Some(close) = block[tag_end..].find("</a>").map(|x| tag_end + x) else { continue };
		let title = decode_entities(&block[tag_end + 1..close]);
		if !mp3_rel.ends_with(".mp3") || title.is_empty() {
			continue;
		}
		let id = href.trim_start_matches("/en/instant/").trim_matches('/').to_owned();
		let mp3 = if mp3_rel.starts_with("http") { mp3_rel.to_owned() } else { format!("{BASE}{mp3_rel}") };
		if !out.iter().any(|s: &OnlineSound| s.mp3 == mp3) {
			out.push(OnlineSound { id, title, mp3 });
		}
	}
	out
}

/// Searches Myinstants (empty query: what's trending). `page` starts at 1.
#[command]
pub async fn sb_online_search(query: String, page: u32) -> Result<OnlinePage, Error> {
	let q = query.trim();
	let mut url = if q.is_empty() {
		format!("{BASE}/en/index/us/")
	} else {
		format!("{BASE}/en/search/?name={}", urlencode(q))
	};
	if page > 1 {
		url.push_str(if url.contains('?') { "&" } else { "?" });
		url.push_str(&format!("page={page}"));
	}
	let html = client()
		.map_err(anyhow::Error::from)?
		.get(&url)
		.send()
		.await
		.and_then(|r| r.error_for_status())
		.map_err(|e| anyhow::anyhow!("Couldn't reach Myinstants ({e})"))?
		.text()
		.await
		.map_err(anyhow::Error::from)?;
	let sounds = parse(&html);
	let more = html.contains(&format!("page={}", page + 1));
	Ok(OnlinePage { sounds, more })
}

fn urlencode(s: &str) -> String {
	s.bytes()
		.map(|b| match b {
			b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
			b' ' => "+".to_string(),
			_ => format!("%{b:02X}"),
		})
		.collect()
}

/// Downloads a Myinstants sound into the soundboard.
#[command]
pub async fn sb_online_add(mp3: String, title: String) -> Result<super::Sound, Error> {
	if !mp3.starts_with("https://www.myinstants.com/") && !mp3.starts_with("https://myinstants.com/") {
		return Err(anyhow::anyhow!("not a Myinstants sound").into());
	}
	let resp = client()
		.map_err(anyhow::Error::from)?
		.get(&mp3)
		.send()
		.await
		.and_then(|r| r.error_for_status())
		.map_err(|e| anyhow::anyhow!("Download failed ({e})"))?;
	if resp.content_length().unwrap_or(0) as usize > MAX_BYTES {
		return Err(anyhow::anyhow!("that sound is too big").into());
	}
	let bytes = resp.bytes().await.map_err(anyhow::Error::from)?;
	if bytes.len() > MAX_BYTES {
		return Err(anyhow::anyhow!("that sound is too big").into());
	}
	let name: String = title.chars().filter(|c| !"\\/:*?\"<>|".contains(*c)).collect();
	let name = format!("{}.mp3", if name.trim().is_empty() { "Sound" } else { name.trim() });
	let sound = tokio::task::spawn_blocking(move || super::add_one(&name, &bytes)).await.map_err(anyhow::Error::from)??;
	super::notify();
	Ok(sound)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn parses_list_pages() {
		let html = r#"
			<div class="instant">
				<button class="small-button" onclick="play('/media/sounds/vine-boom.mp3', 'loader-1', 'vine-boom-1')"></button>
				<a href="/en/instant/vine-boom/" class="instant-link link-secondary">Vine Boom &amp; Bass</a>
			</div>
			<div class="instant">
				<button class="small-button" onclick="play('/media/sounds/bruh.mp3')"></button>
				<a class="instant-link" href="/en/instant/bruh-sound-effect/">Bruh &#233;</a>
			</div>"#;
		let s = parse(html);
		assert_eq!(s.len(), 2);
		assert_eq!(s[0].title, "Vine Boom & Bass");
		assert_eq!(s[0].mp3, "https://www.myinstants.com/media/sounds/vine-boom.mp3");
		assert_eq!(s[0].id, "vine-boom");
		assert_eq!(s[1].title, "Bruh é");
		assert_eq!(s[1].id, "bruh-sound-effect");
	}
}
