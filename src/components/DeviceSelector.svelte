<script lang="ts">
	import type { DeviceInfo } from "$lib/DeviceInfo";
	import type { Profile } from "$lib/Profile";

	import { t } from "$lib/i18n";
	import { profileManager } from "$lib/singletons";

	import { invoke } from "@tauri-apps/api/core";
	import { listen } from "@tauri-apps/api/event";
	import { getCurrentWindow, LogicalPosition, LogicalSize } from "@tauri-apps/api/window";

	export let devices: { [id: string]: DeviceInfo } = {};
	export let value: string;
	export let selectedProfiles: { [id: string]: Profile } = {};

	let registered: string[] = [];
	$: {
		if (!value || !devices[value]) value = Object.keys(devices).sort()[0];
		for (const [id, device] of Object.entries(devices)) {
			if (!registered.includes(id)) {
				(async () => {
					let profile: Profile = await invoke("get_selected_profile", { device: device.id });
					selectedProfiles[id] = profile;
					await invoke("set_selected_profile", { device: id, id: profile.id });
				})();
				registered.push(id);
			}
		}
	}

	export function reloadProfiles() {
		registered = [];
	}

	listen("switch_profile", async ({ payload }: { payload: { device: string; profile: string } }) => {
		if (payload.device == value) {
			$profileManager?.setProfile(payload.profile);
		} else {
			await invoke("set_selected_profile", { device: payload.device, id: payload.profile });
			selectedProfiles[payload.device] = await invoke("get_selected_profile", { device: payload.device });
		}
	});

	(async () => (devices = await invoke("get_devices")))();
	listen("devices", ({ payload }: { payload: { [id: string]: DeviceInfo } }) => (devices = payload));

	let buildInfo: string;
	(async () => (buildInfo = await invoke("get_build_info")))();
	const window = getCurrentWindow();

	// Window: a fixed minimum size set once, and the window comes back the way it was left
	// (maximised by default). Connecting / disconnecting the pad no longer touches the window.
	(async () => {
		const minW = Math.min(1180, screen.availWidth),
			minH = Math.min(760, screen.availHeight);
		await window.setMinSize(new LogicalSize(minW, minH));
		let saved: { max: boolean; w?: number; h?: number; x?: number; y?: number } = { max: true };
		try {
			saved = JSON.parse(localStorage.getItem("orvo-window") ?? "") ?? saved;
		} catch {}
		const restore = async () => {
			if (saved.max) await window.maximize();
			else if (saved.w && saved.h) {
				await window.setSize(new LogicalSize(Math.max(saved.w, minW), Math.max(saved.h, minH)));
				if (saved.x != null && saved.y != null) await window.setPosition(new LogicalPosition(saved.x, saved.y));
			}
		};
		if (await window.isVisible()) await restore();
		else {
			const off = await window.onFocusChanged(({ payload }) => {
				if (payload) {
					off();
					restore();
				}
			});
		}
		let timer: number | null = null;
		const remember = () => {
			if (timer) clearTimeout(timer);
			timer = setTimeout(async () => {
				if (await window.isMinimized()) return;
				const max = await window.isMaximized();
				const scale = await window.scaleFactor();
				const size = (await window.innerSize()).toLogical(scale);
				const pos = (await window.outerPosition()).toLogical(scale);
				const state = max ? { ...saved, max: true } : { max: false, w: size.width, h: size.height, x: pos.x, y: pos.y };
				saved = state;
				try {
					localStorage.setItem("orvo-window", JSON.stringify(state));
				} catch {}
			}, 400) as unknown as number;
		};
		await window.onResized(remember);
		await window.onMoved(remember);
	})();

	let measure: HTMLSpanElement;
	let selectWidth = 0;
	$: if (value && measure && devices[value]) {
		measure.textContent = devices[value].name;
		selectWidth = measure.offsetWidth + 20;
	}
</script>

{#if Object.keys(devices).length > 0}
	<div class="select-device-wrapper">
		<span bind:this={measure} class="invisible fixed whitespace-pre pointer-events-none text-base font-semibold" aria-hidden="true"></span>
		<select bind:value style:width="{selectWidth}px" aria-label={$t("device_selector.device")}>
			<option value="" disabled selected>{$t("device_selector.choose_device")}</option>

			{#each Object.entries(devices).sort() as [id, device]}
				<option value={id}>{device.name}</option>
			{/each}
		</select>
	</div>
{/if}
