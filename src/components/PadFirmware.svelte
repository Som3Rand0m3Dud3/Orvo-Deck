<script lang="ts" context="module">
	import { writable } from "svelte/store";
	// Kept outside the component so progress survives the pad disconnecting while it updates.
	const store = writable<{ [device: string]: any }>({});
</script>

<script lang="ts">
	// "Pad" card: shows the pad's firmware version and installs new firmware with one click.
	// The NineDeck plugin does the work and reports progress with "pad_status" events.
	import { invoke } from "@tauri-apps/api/core";
	import { listen } from "@tauri-apps/api/event";
	import { ask } from "$lib/dialog";
	import { onMount } from "svelte";

	export let device: string;

	type PadStatus = { device: string; name: string; firmware: string; latest: string; updateAvailable: boolean; state: "ready" | "updating" | "done" | "error"; message: string };
	$: statuses = $store as { [device: string]: PadStatus };

	let autoUpdate = false;
	onMount(() => {
		// After Orvo Studio updated itself, update the pad straight away too (one click overall).
		invoke<boolean>("orvo_take_update_marker").then((v) => (autoUpdate = v)).catch(() => {});
		// Tell the pad plugin the chosen slide speed.
		try {
			const saved = localStorage.getItem("orvo-slide-ms");
			const taper = localStorage.getItem("orvo-slider-taper");
			invoke("orvo_pad_settings", {
				slideMs: saved != null ? parseInt(saved) || 0 : 180,
				taper: taper == "1" ? 1 : 0,
				flip: localStorage.getItem("orvo-slider-flip") == "1",
			}).catch(() => {});
		} catch {}
		const unlisten = listen("pad_status", ({ payload }: { payload: PadStatus }) => {
			store.update((s) => ({ ...s, [payload.device]: payload }));
			if (autoUpdate && payload.updateAvailable && payload.state == "ready") {
				autoUpdate = false;
				startUpdate(payload);
			}
		});
		invoke("request_pad_status").catch(() => {});
		return () => unlisten.then((f) => f());
	});

	$: status = statuses[device];

	async function startUpdate(target: PadStatus) {
		store.update((s) => ({ ...s, [target.device]: { ...target, state: "updating", message: "Updating the pad to match the new version…" } }));
		try {
			await invoke("update_pad_firmware", { device: target.device });
		} catch (error) {
			store.update((s) => ({ ...s, [target.device]: { ...target, state: "error", message: String(error) } }));
		}
	}

	async function update() {
		if (!status) return;
		const ok = await ask(
			`Install firmware v${status.latest} on your ${status.name}?\n\nThe pad's screen goes blank for a few seconds while it updates. Keep it plugged in until it comes back.`,
			{ title: "Update pad", kind: "info" },
		);
		if (!ok) return;
		store.update((s) => ({ ...s, [device]: { ...status, state: "updating", message: "Starting…" } }));
		try {
			await invoke("update_pad_firmware", { device });
		} catch (error) {
			store.update((s) => ({ ...s, [device]: { ...status, state: "error", message: String(error) } }));
		}
	}
</script>

{#if status}
	<div class="mx-3 mb-3 p-3 rounded-xl bg-neutral-800 border border-neutral-700">
		<div class="flex items-center justify-between gap-2">
			<div class="min-w-0">
				<p class="text-sm font-semibold text-neutral-100">{status.name} firmware</p>
				<p class="text-xs text-neutral-400">v{status.firmware || "?"}{status.updateAvailable && status.state != "updating" ? ` · v${status.latest} available` : status.state == "ready" ? " · up to date" : ""}</p>
			</div>
			{#if status.updateAvailable && status.state != "updating"}
				<button class="shrink-0 px-3 h-9 rounded-lg text-sm font-semibold text-white" style="background: var(--orvo-accent)" on:click={update}>Update pad</button>
			{/if}
		</div>
		{#if status.state == "updating"}
			<p class="mt-2 text-xs text-neutral-300 animate-pulse">{status.message}</p>
		{:else if status.state == "done"}
			<p class="mt-2 text-xs text-emerald-400">{status.message}</p>
		{:else if status.state == "error"}
			<p class="mt-2 text-xs text-red-400">{status.message}</p>
		{/if}
	</div>
{/if}
