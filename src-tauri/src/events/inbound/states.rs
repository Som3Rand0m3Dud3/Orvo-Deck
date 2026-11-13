use super::ContextAndPayloadEvent;

use crate::events::frontend::instances::update_state;
use crate::shared::{ActionContext, DEVICES};
use crate::store::profiles::{LocksMut, acquire_locks_mut, get_device_profiles, get_instance_mut, mark_profile_stale};

use anyhow::bail;
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
pub struct SetTitlePayload {
	title: Option<String>,
	state: Option<u16>,
}

#[derive(Deserialize)]
pub struct SetImagePayload {
	image: Option<String>,
	state: Option<u16>,
}

#[derive(Deserialize)]
pub struct SetStatePayload {
	state: u16,
}

#[derive(Debug, Deserialize)]
pub struct SetFeedbackLayoutPayload {
	layout: String,
}

pub async fn set_title(event: ContextAndPayloadEvent<SetTitlePayload>) -> Result<(), anyhow::Error> {
	let mut locks = acquire_locks_mut().await;

	if let Some(instance) = get_instance_mut(&event.context, &mut locks).await? {
		if let Some(state) = event.payload.state {
			if state as usize >= instance.states.len() {
				return Err(anyhow::anyhow!("State index out of bounds ({} > {})", state, instance.states.len() - 1));
			}

			let text = event.payload.title.unwrap_or(instance.action.states[state as usize].text.clone());
			if instance.states[state as usize].text == text {
				return Ok(());
			}
			instance.states[state as usize].text = text;
		} else {
			if instance
				.states
				.iter()
				.enumerate()
				.all(|(index, state)| state.text == event.payload.title.clone().unwrap_or(instance.action.states[index].text.clone()))
			{
				return Ok(());
			}

			for (index, state) in instance.states.iter_mut().enumerate() {
				state.text = event.payload.title.clone().unwrap_or(instance.action.states[index].text.clone());
			}
		}
		update_state(crate::APP_HANDLE.get().unwrap(), instance.context.clone(), &mut locks).await?;
		mirror_identical(&event.context, &mut locks).await;
	}
	mark_profile_stale(&event.context.device, &mut locks).await?;

	Ok(())
}

pub async fn set_image(mut event: ContextAndPayloadEvent<SetImagePayload>) -> Result<(), anyhow::Error> {
	let mut locks = acquire_locks_mut().await;

	if let Some(instance) = get_instance_mut(&event.context, &mut locks).await? {
		if let Some(image) = &event.payload.image {
			if image.trim().is_empty() {
				event.payload.image = None;
			} else if !image.trim().starts_with("data:") {
				event.payload.image = Some(crate::shared::convert_icon(
					crate::shared::config_dir()
						.join("plugins")
						.join(&instance.action.plugin)
						.join(image.trim())
						.to_str()
						.unwrap()
						.to_owned(),
				));
			}
		}

		if let Some(state) = event.payload.state {
			if state as usize >= instance.states.len() {
				return Err(anyhow::anyhow!("State index out of bounds ({} > {})", state, instance.states.len() - 1));
			}
			instance.states[state as usize].image = event.payload.image.clone().unwrap_or(instance.action.states[state as usize].image.clone());
		} else {
			for (index, state) in instance.states.iter_mut().enumerate() {
				state.image = event.payload.image.clone().unwrap_or(instance.action.states[index].image.clone());
			}
		}
		update_state(crate::APP_HANDLE.get().unwrap(), instance.context.clone(), &mut locks).await?;
		mirror_identical(&event.context, &mut locks).await;
	}

	mark_profile_stale(&event.context.device, &mut locks).await?;
	Ok(())
}

pub async fn set_feedback(event: ContextAndPayloadEvent<Value>) -> Result<(), anyhow::Error> {
	let mut locks = acquire_locks_mut().await;

	if let Some(instance) = get_instance_mut(&event.context, &mut locks).await?
		&& let Some(encoder) = &mut instance.action.encoder
	{
		let Some(layout) = &mut encoder.layout_parsed else {
			bail!("Layout is not loaded; cannot set feedback");
		};

		layout.set_feedback(event.payload)?;
		update_state(crate::APP_HANDLE.get().unwrap(), instance.context.clone(), &mut locks).await?;
	}

	Ok(())
}

pub async fn set_feedback_layout(event: ContextAndPayloadEvent<SetFeedbackLayoutPayload>) -> Result<(), anyhow::Error> {
	let mut locks = acquire_locks_mut().await;
	if let Some(instance) = get_instance_mut(&event.context, &mut locks).await? {
		// We need to replace the existing parsed layout with the new one
		let layout_name = event.payload.layout.clone();
		crate::shared::initialise_encoder_layout(&mut instance.action, Some(layout_name))?;

		// Trigger a state update; should cause a redraw
		update_state(crate::APP_HANDLE.get().unwrap(), instance.context.clone(), &mut locks).await?;
	}
	Ok(())
}

pub async fn set_state(event: ContextAndPayloadEvent<SetStatePayload>) -> Result<(), anyhow::Error> {
	let mut locks = acquire_locks_mut().await;

	if let Some(instance) = get_instance_mut(&event.context, &mut locks).await? {
		if event.payload.state >= instance.states.len() as u16 {
			return Ok(());
		}
		instance.current_state = event.payload.state;
		update_state(crate::APP_HANDLE.get().unwrap(), instance.context.clone(), &mut locks).await?;
		mirror_identical(&event.context, &mut locks).await;
	}
	mark_profile_stale(&event.context.device, &mut locks).await?;

	Ok(())
}

/// Orvo: the same action (same plugin action and same settings) can sit on several pages.
/// Plugins usually only update the copy on the page that is showing, so copy its live
/// look (current state, plugin image and title) to every identical copy on the device's
/// other pages. This keeps e.g. Discord mute ON on every page at once.
async fn mirror_identical(context: &ActionContext, locks: &mut LocksMut<'_>) {
	if let Err(error) = mirror_inner(context, locks).await {
		log::warn!("Failed to sync identical actions: {error}");
	}
}

async fn mirror_inner(context: &ActionContext, locks: &mut LocksMut<'_>) -> Result<(), anyhow::Error> {
	if context.index != 0 {
		return Ok(());
	}
	let Some(source) = get_instance_mut(context, locks).await?.cloned() else { return Ok(()) };
	let Some(device) = DEVICES.get(&context.device).map(|d| d.clone()) else { return Ok(()) };

	let mut ids = get_device_profiles(&context.device)?;
	ids.sort();
	ids.dedup();

	let mut touched = vec![];
	for id in ids {
		let Ok(store) = locks.profile_stores.get_profile_store_mut(&device, &id).await else { continue };
		let mut changed = id == context.profile;
		for slot in store.value.keys.iter_mut().chain(store.value.sliders.iter_mut()).flatten() {
			if slot.context == source.context
				|| slot.action.uuid != source.action.uuid
				|| slot.action.plugin != source.action.plugin
				|| slot.settings != source.settings
				|| slot.states.len() != source.states.len()
			{
				continue;
			}
			slot.current_state = source.current_state;
			for (dest, src) in slot.states.iter_mut().zip(source.states.iter()) {
				dest.image = src.image.clone();
				dest.text = src.text.clone();
			}
			touched.push(slot.context.clone());
			changed = true;
		}
		if changed {
			store.value.stale = true;
		}
	}

	for context in touched {
		update_state(crate::APP_HANDLE.get().unwrap(), context, locks).await?;
	}
	Ok(())
}
