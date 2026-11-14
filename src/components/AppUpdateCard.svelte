<script lang="ts">
	// Orvo Studio card above the pad firmware card: checks for a new version every 5 minutes.
	import { appUpdate } from "$lib/orvoUpdate";
	import { invoke } from "@tauri-apps/api/core";
	import { ask, message } from "@tauri-apps/plugin-dialog";
	import { onDestroy, onMount } from "svelte";

	type Release = { tag: string; newer: boolean; current: boolean };
	let current = "";
	let state: "checking" | "idle" | "error" | "installing" = "checking";

	async function check() {
		let test = true;
		try {
			const saved = localStorage.getItem("orvo-test-versions");
			if (saved != null) test = saved == "1";
		} catch {}
		try {
			const releases = await invoke<Release[]>("list_releases", { includePrerelease: test });
			const newer = releases.find((r) => r.newer);
			current = releases.find((r) => r.current)?.tag ?? current;
			appUpdate.set(newer ? { tag: newer.tag } : null);
			if (state != "installing") state = "idle";
		} catch {
			if (state != "installing") state = "error";
		}
	}

	async function install() {
		const tag = $appUpdate?.tag;
		if (!tag) return;
		if (!(await ask(`Update Orvo Studio to ${tag}? It will close, update and open again.`, { title: "Orvo Studio update", kind: "info" }))) return;
		state = "installing";
		try {
			await invoke("install_release", { tag });
		} catch (error) {
			state = "error";
			await message(String(error), { title: "Orvo Studio update", kind: "error" });
		}
	}

	let timer: ReturnType<typeof setInterval>;
	onMount(async () => {
		try {
			const info: string = await invoke("get_build_info");
			current = "v" + (info.match(/v(\d+\.\d+\.\d+[^\s<]*)/)?.[1] ?? "");
		} catch {}
		check();
		timer = setInterval(check, 5 * 60 * 1000);
	});
	onDestroy(() => clearInterval(timer));
</script>

<div class="mx-3 mb-2 p-3 rounded-xl bg-neutral-800 border border-neutral-700">
	<div class="flex items-center justify-between gap-2">
		<div class="min-w-0">
			<p class="text-sm font-semibold text-neutral-100">Orvo Studio</p>
			<p class="text-xs {$appUpdate ? 'text-emerald-400' : 'text-neutral-400'}">
				{current || "?"}{state == "checking" ? " · checking…" : state == "installing" ? " · updating…" : $appUpdate ? ` · ${$appUpdate.tag} available` : state == "error" ? " · couldn't check" : " · up to date"}
			</p>
		</div>
		{#if $appUpdate && state != "installing"}
			<button class="shrink-0 px-3 h-9 rounded-lg text-sm font-semibold text-white" style="background: var(--orvo-accent)" on:click={install}>Update</button>
		{/if}
	</div>
</div>
