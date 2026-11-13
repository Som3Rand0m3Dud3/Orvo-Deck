<script lang="ts">
	// Orvo: draws every page of a NineDeck in the background and hands the pictures to the
	// pad plugin, so the pad's ◀ ▶ keys can switch pages instantly (any number of pages).
	import type { ActionInstance } from "$lib/ActionInstance";
	import type { Profile } from "$lib/Profile";

	import { renderImage, wantsVolumeBadge } from "$lib/rendererHelper";
	import { settings } from "$lib/settings";

	import { invoke } from "@tauri-apps/api/core";
	import { listen } from "@tauri-apps/api/event";
	import { onDestroy } from "svelte";

	export let device: string;
	export let profile: Profile | undefined; // the page showing now; any change re-draws the pages

	let timer: ReturnType<typeof setTimeout> | null = null;
	let running = false;
	let again = false;

	function schedule(delay = 700) {
		if (!device?.startsWith("nd-")) return;
		if (timer) clearTimeout(timer);
		timer = setTimeout(run, delay);
	}

	async function picture(instance: ActionInstance | null): Promise<string | null> {
		if (!instance) return null;
		const index = instance.current_state ?? 0;
		const state = instance.states[index];
		if (!state) return null;
		const fallback = instance.action.states[index]?.image ?? instance.action.icon;
		const canvas = await renderImage(null, null, state, fallback, false, false, true, false, false, $settings?.rotation, wantsVolumeBadge(instance.action.uuid, state.image_override));
		if (!canvas) return null;
		// see-through parts show the pad's box colour (JPEG has no transparency)
		const flat = document.createElement("canvas");
		flat.width = canvas.width;
		flat.height = canvas.height;
		const fc = flat.getContext("2d");
		if (!fc) return canvas.toDataURL("image/jpeg");
		fc.fillStyle = "#181c18";
		fc.fillRect(0, 0, flat.width, flat.height);
		fc.drawImage(canvas, 0, 0);
		return flat.toDataURL("image/jpeg");
	}

	async function run() {
		timer = null;
		if (running) {
			again = true;
			return;
		}
		running = true;
		try {
			const pages: string[] = await invoke("get_profiles", { device });
			for (const id of [...new Set(pages)]) {
				const page: Profile = await invoke("orvo_get_profile", { device, id });
				const images: (string | null)[] = [];
				for (let i = 0; i < 9; i++) images.push(await picture(page.keys[i] ?? null));
				await invoke("orvo_page_images", { device, profile: id, images });
			}
		} catch (error) {
			console.warn("Orvo: could not prepare pad pages", error);
		}
		running = false;
		if (again) {
			again = false;
			schedule(200);
		}
	}

	$: if (device) schedule(profile ? 700 : 1500);
	$: if (profile) schedule();

	const unlisteners = [listen("update_state", () => schedule(1500)), listen("rerender_images", () => schedule()), listen("devices", () => schedule(1500))];
	onDestroy(() => {
		if (timer) clearTimeout(timer);
		unlisteners.forEach((u) => u.then((f) => f()));
	});
</script>
