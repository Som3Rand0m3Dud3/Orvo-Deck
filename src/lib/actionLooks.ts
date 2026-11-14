// Orvo: your own picture and name for an action in the actions panel. Every key you put
// that action on gets them. Kept by Orvo Studio (not on the pad).
import type { ActionInstance } from "./ActionInstance";
import type { SoundAction } from "./soundboard";

import { invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";
import { get, writable } from "svelte/store";

export type Look = { image?: string; name?: string };
export const actionLooks = writable<{ [key: string]: Look }>({});

invoke<{ [key: string]: Look } | null>("orvo_kv_get", { key: "action-looks" })
	.then((v) => v && typeof v == "object" && actionLooks.set(v))
	.catch(() => {});

/** One key per cube: the action, plus the sound or voice it carries. */
export function lookKey(a: SoundAction): string {
	return [a.plugin, a.uuid, a.orvoSound?.id ?? "", a.orvoPreset?.settings?.preset ?? ""].join("|");
}

export async function saveLook(key: string, look: Look | null) {
	const all = { ...get(actionLooks) };
	if (look && (look.image || look.name)) all[key] = look;
	else delete all[key];
	actionLooks.set(all);
	await invoke("orvo_kv_set", { key: "action-looks", value: all });
}

/** Turns any picture file into a 144×144 PNG (what keys and the pad's screen use). */
export function toKeyPicture(file: File): Promise<string> {
	return new Promise((resolve, reject) => {
		const url = URL.createObjectURL(file);
		const img = new Image();
		img.onload = () => {
			const c = document.createElement("canvas");
			c.width = c.height = 144;
			const ctx = c.getContext("2d")!;
			const s = Math.min(img.width, img.height);
			ctx.drawImage(img, (img.width - s) / 2, (img.height - s) / 2, s, s, 0, 0, 144, 144);
			URL.revokeObjectURL(url);
			resolve(c.toDataURL("image/png"));
		};
		img.onerror = () => {
			URL.revokeObjectURL(url);
			reject(new Error("That file isn't a picture Orvo can read."));
		};
		img.src = url;
	});
}

/** Gives a key the look: picture on every state, and the name as its title if one was set. */
export async function applyLook(instance: ActionInstance, look: Look | undefined, title?: string): Promise<ActionInstance> {
	if (!look?.image && !look?.name && title == undefined) return instance;
	for (let i = 0; i < instance.states.length; i++) {
		const state = { ...instance.states[i] };
		if (look?.image) state.image = look.image;
		if (look?.name != undefined) state.text = look.name;
		else if (title != undefined) state.text = title;
		state.show = true;
		try {
			await invoke("set_state", { context: instance.context, index: i, state });
			instance.states[i] = state;
		} catch {}
	}
	return instance;
}

/** Same look for keys that already use this action, on every page of every pad. */
export async function updatePlacedKeys(match: (a: ActionInstance) => boolean, lookFor: (a: ActionInstance) => Look | undefined): Promise<number> {
	let count = 0;
	const devices = await invoke<{ [id: string]: unknown }>("get_devices").catch(() => ({}));
	for (const device of Object.keys(devices)) {
		const pages: string[] = await invoke<string[]>("get_profiles", { device }).catch(() => []);
		for (const id of new Set(pages)) {
			const page: any = await invoke("orvo_get_profile", { device, id }).catch(() => null);
			for (const slot of [...(page?.keys ?? []), ...(page?.sliders ?? []), ...(page?.encoders ?? [])]) {
				if (slot && match(slot)) {
					await applyLook(slot, lookFor(slot));
					count++;
				}
			}
		}
	}
	if (count) emit("rerender_images", null).catch(() => {});
	return count;
}
