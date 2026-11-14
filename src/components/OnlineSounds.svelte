<script lang="ts">
	// Find sounds on Myinstants and add them to the soundboard with one click.
	import Check from "phosphor-svelte/lib/Check";
	import MagnifyingGlass from "phosphor-svelte/lib/MagnifyingGlass";
	import Play from "phosphor-svelte/lib/Play";
	import Plus from "phosphor-svelte/lib/Plus";
	import Stop from "phosphor-svelte/lib/Stop";
	import X from "phosphor-svelte/lib/X";

	import { refreshSoundboard } from "$lib/soundboard";

	import { invoke } from "@tauri-apps/api/core";
	import { createEventDispatcher, onDestroy, onMount } from "svelte";

	type OnlineSound = { id: string; title: string; mp3: string };
	const dispatch = createEventDispatcher<{ close: void }>();

	let query = "";
	let searched = "";
	let results: OnlineSound[] = [];
	let page = 1;
	let more = false;
	let loading = false;
	let error = "";
	let added = new Map<string, "adding" | "added" | string>();

	async function search(next = false) {
		loading = true;
		error = "";
		if (!next) {
			page = 1;
			searched = query.trim();
		}
		try {
			const res = await invoke<{ sounds: OnlineSound[]; more: boolean }>("sb_online_search", { query: searched, page });
			results = next ? [...results, ...res.sounds.filter((s) => !results.some((r) => r.mp3 == s.mp3))] : res.sounds;
			more = res.more && res.sounds.length > 0;
		} catch (e) {
			error = String(e);
			if (!next) results = [];
		}
		loading = false;
	}
	onMount(() => search());

	// preview: plays in the app (your default speakers), not into the mic
	let audio: HTMLAudioElement | null = null;
	let playingUrl: string | null = null;
	function preview(s: OnlineSound) {
		if (playingUrl == s.mp3) return stop();
		stop();
		audio = new Audio(s.mp3);
		audio.volume = 0.8;
		audio.onended = stop;
		audio.onerror = () => {
			error = `Couldn't play "${s.title}".`;
			stop();
		};
		audio.play().catch(() => {});
		playingUrl = s.mp3;
	}
	function stop() {
		audio?.pause();
		audio = null;
		playingUrl = null;
	}
	onDestroy(stop);

	async function add(s: OnlineSound) {
		added.set(s.mp3, "adding");
		added = added;
		try {
			await invoke("sb_online_add", { mp3: s.mp3, title: s.title });
			added.set(s.mp3, "added");
			refreshSoundboard();
		} catch (e) {
			added.set(s.mp3, String(e));
		}
		added = added;
	}
</script>

<svelte:window on:keydown={(e) => e.key == "Escape" && dispatch("close")} />

<!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
<div class="fixed inset-0 z-40 bg-black/55" on:click={() => dispatch("close")}></div>
<div
	role="dialog"
	aria-label="Find sounds online"
	class="fixed left-1/2 top-1/2 -translate-x-1/2 -translate-y-1/2 z-50 w-[56rem] max-w-[calc(100%-2rem)] h-[44rem] max-h-[calc(100%-2rem)] flex flex-col gap-3 p-5 rounded-2xl bg-neutral-900 border border-neutral-600 shadow-2xl"
>
	<div class="flex items-center gap-3">
		<div class="grow">
			<h2 class="text-lg font-bold text-neutral-100">Find sounds online</h2>
			<p class="text-sm text-neutral-400">From Myinstants. ▶ to listen, + to add it to your soundboard.</p>
		</div>
		<button class="p-2 rounded-lg text-neutral-400 hover:text-neutral-100 hover:bg-neutral-800" aria-label="Close" on:click={() => dispatch("close")}
			><X size="18" /></button
		>
	</div>
	<form class="flex gap-2" on:submit|preventDefault={() => search()}>
		<label class="flex items-center grow h-10 px-3 gap-2 rounded-lg bg-neutral-800 border border-neutral-600">
			<MagnifyingGlass size="16" class="text-neutral-400" />
			<!-- svelte-ignore a11y-autofocus -->
			<input
				bind:value={query}
				autofocus
				class="grow bg-transparent text-neutral-100"
				placeholder="Search sounds: bruh, vine boom, airhorn…"
				type="search"
				spellcheck="false"
			/>
		</label>
		<button type="submit" class="px-4 h-10 rounded-lg text-sm font-semibold text-white" style="background: var(--orvo-accent)">Search</button>
	</form>
	<p class="text-xs text-neutral-500">{searched ? `Results for "${searched}"` : "Trending now"}</p>

	<div class="grow min-h-0 overflow-y-auto pr-1">
		{#if error}<p class="mb-2 text-sm text-red-400">{error}</p>{/if}
		<div class="grid gap-2" style="grid-template-columns: repeat(auto-fill, minmax(240px, 1fr))">
			{#each results as s (s.mp3)}
				{@const state = added.get(s.mp3)}
				<div
					class="flex items-center gap-2 p-2 rounded-xl bg-[#1C1F26] border {playingUrl == s.mp3 ? '' : 'border-neutral-700'}"
					style={playingUrl == s.mp3 ? "border-color: var(--orvo-accent)" : ""}
				>
					<button
						class="flex items-center justify-center shrink-0 w-10 h-10 rounded-lg text-white"
						style="background: {playingUrl == s.mp3 ? 'var(--orvo-accent)' : 'color-mix(in srgb, var(--orvo-accent) 22%, #262626)'}"
						aria-label="{playingUrl == s.mp3 ? 'Stop' : 'Listen to'} {s.title}"
						on:click={() => preview(s)}
					>
						{#if playingUrl == s.mp3}<Stop size="16" weight="fill" />{:else}<Play size="16" weight="fill" />{/if}
					</button>
					<span class="grow min-w-0 text-sm text-neutral-100 truncate" title={s.title}>{s.title}</span>
					{#if state == "added"}
						<span class="flex items-center gap-1 shrink-0 px-2 h-9 text-sm text-emerald-400"><Check size="16" weight="bold" /> Added</span>
					{:else if state == "adding"}
						<span class="shrink-0 px-2 h-9 flex items-center text-sm text-neutral-400 animate-pulse">Adding…</span>
					{:else}
						<button
							class="flex items-center justify-center shrink-0 w-9 h-9 rounded-lg border border-neutral-600 text-neutral-200 hover:bg-neutral-700"
							aria-label="Add {s.title}"
							title={state ? `Couldn't add: ${state}` : "Add to my soundboard"}
							on:click={() => add(s)}><Plus size="16" weight="bold" class={state ? "text-red-400" : ""} /></button
						>
					{/if}
				</div>
			{:else}
				{#if !loading}<p class="col-span-full py-10 text-center text-sm text-neutral-400">{error ? "" : "Nothing found. Try other words."}</p>{/if}
			{/each}
		</div>
		{#if loading}<p class="py-4 text-center text-sm text-neutral-400 animate-pulse">Loading…</p>{/if}
		{#if more && !loading}
			<div class="flex justify-center py-3">
				<button
					class="px-4 h-9 rounded-lg text-sm text-neutral-200 bg-neutral-800 border border-neutral-600 hover:bg-neutral-700"
					on:click={() => {
						page += 1;
						search(true);
					}}>Load more</button
				>
			</div>
		{/if}
	</div>
</div>
