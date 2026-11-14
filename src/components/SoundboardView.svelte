<script lang="ts">
	// Orvo Soundboard page: your sounds, and how they reach your virtual mic and headphones.
	import ArrowClockwise from "phosphor-svelte/lib/ArrowClockwise";
	import DotsThreeVertical from "phosphor-svelte/lib/DotsThreeVertical";
	import Headphones from "phosphor-svelte/lib/Headphones";
	import MagnifyingGlass from "phosphor-svelte/lib/MagnifyingGlass";
	import Microphone from "phosphor-svelte/lib/Microphone";
	import Play from "phosphor-svelte/lib/Play";
	import Plus from "phosphor-svelte/lib/Plus";
	import SpeakerHigh from "phosphor-svelte/lib/SpeakerHigh";
	import Stop from "phosphor-svelte/lib/Stop";
	import Trash from "phosphor-svelte/lib/Trash";
	import UploadSimple from "phosphor-svelte/lib/UploadSimple";

	import { ask, message } from "$lib/dialog";
	import { refreshSoundboard, soundboard, type Sound, type SoundboardConfig } from "$lib/soundboard";

	import { invoke } from "@tauri-apps/api/core";
	import { open } from "@tauri-apps/plugin-dialog";
	import { onDestroy, onMount } from "svelte";

	const EXTENSIONS = ["mp3", "wav", "ogg", "oga", "flac", "m4a", "aac"];

	$: state = $soundboard;
	$: config = state?.config;
	$: ready = !!state?.cable_installed;

	onMount(() => {
		// Device lists change (headsets plugged in, VB-Cable installed): re-read when the page opens.
		refreshSoundboard();
	});

	// ---------- playing look (polls only while something plays)
	let playing = new Set<string>();
	let poll: ReturnType<typeof setInterval> | null = null;
	function watchPlaying() {
		if (poll) return;
		poll = setInterval(async () => {
			const ids: string[] = await invoke<string[]>("sb_playing").catch(() => []);
			playing = new Set(ids);
			if (ids.length == 0 && poll) {
				clearInterval(poll);
				poll = null;
			}
		}, 150);
	}
	onDestroy(() => poll && clearInterval(poll));

	async function play(sound: Sound) {
		try {
			await invoke("sb_play", { id: sound.id });
			playing = new Set([...playing, sound.id]);
			watchPlaying();
		} catch (error) {
			message(String(error), { title: `Couldn't play "${sound.name}"` });
		}
	}
	async function stopAll() {
		await invoke("sb_stop_all");
		playing = new Set();
	}

	// ---------- settings
	let saveTimer: ReturnType<typeof setTimeout> | null = null;
	function setConfig(patch: Partial<SoundboardConfig>) {
		if (!state || !config) return;
		const next = { ...config, ...patch };
		soundboard.set({ ...state, config: next });
		if (saveTimer) clearTimeout(saveTimer);
		saveTimer = setTimeout(async () => {
			await invoke("sb_set_config", { config: next });
			setTimeout(refreshSoundboard, 400);
		}, 120);
	}
	const pct = (v: number) => Math.round(v * 100);

	// ---------- adding sounds
	let adding = 0;
	let dragOver = 0;
	const isSound = (name: string) => EXTENSIONS.includes(name.split(".").pop()?.toLowerCase() ?? "");

	async function addFromDialog() {
		const picked = await open({ multiple: true, directory: false, filters: [{ name: "Sounds", extensions: EXTENSIONS }] });
		if (!picked) return;
		const paths = Array.isArray(picked) ? picked : [picked];
		adding += paths.length;
		try {
			const result = await invoke<{ added: Sound[]; failed: string[] }>("sb_add_files", { paths });
			report(result.failed);
		} finally {
			adding -= paths.length;
			refreshSoundboard();
		}
	}

	function report(failed: string[]) {
		if (failed.length)
			message(`These couldn't be added:\n\n${failed.slice(0, 8).join("\n")}${failed.length > 8 ? `\n…and ${failed.length - 8} more` : ""}`, {
				title: "Some files were skipped",
			});
	}

	async function filesFromEntry(entry: any): Promise<File[]> {
		if (entry.isFile)
			return new Promise((resolve) =>
				entry.file(
					(f: File) => resolve([f]),
					() => resolve([]),
				),
			);
		if (!entry.isDirectory) return [];
		const reader = entry.createReader();
		const all: File[] = [];
		for (;;) {
			const batch: any[] = await new Promise((resolve) => reader.readEntries(resolve, () => resolve([])));
			if (!batch.length) break;
			for (const e of batch) all.push(...(await filesFromEntry(e)));
		}
		return all;
	}

	async function onDrop(event: DragEvent) {
		dragOver = 0;
		const dt = event.dataTransfer;
		if (!dt || !dt.types.includes("Files")) return;
		event.preventDefault();
		let files: File[] = [];
		const entries = [...dt.items].map((i) => i.webkitGetAsEntry?.()).filter(Boolean);
		if (entries.length) for (const e of entries) files.push(...(await filesFromEntry(e)));
		else files = [...dt.files];
		const sounds = files.filter((f) => isSound(f.name));
		const failed = files.filter((f) => !isSound(f.name)).map((f) => `${f.name}: not a sound file`);
		adding += sounds.length;
		for (const file of sounds) {
			try {
				const bytes = new Uint8Array(await file.arrayBuffer());
				await invoke("sb_add_bytes", bytes, { headers: { "x-name": encodeURIComponent(file.name) } });
			} catch (error) {
				failed.push(`${file.name}: ${error}`);
			} finally {
				adding--;
			}
		}
		refreshSoundboard();
		report(failed);
	}
	function onDragEnter(event: DragEvent) {
		if (event.dataTransfer?.types.includes("Files")) dragOver++;
	}
	function onDragOver(event: DragEvent) {
		if (event.dataTransfer?.types.includes("Files")) {
			event.preventDefault();
			event.dataTransfer.dropEffect = "copy";
		}
	}

	// ---------- editing one sound
	let editing: Sound | null = null;
	let editSave: ReturnType<typeof setTimeout> | null = null;
	function edit(patch: Partial<Sound>) {
		if (!editing || !state) return;
		editing = { ...editing, ...patch };
		const e = editing;
		soundboard.set({ ...state, sounds: state.sounds.map((s) => (s.id == e.id ? e : s)) });
		if (editSave) clearTimeout(editSave);
		editSave = setTimeout(() => invoke("sb_update", { sound: e }), 200);
	}
	const setMode = (mode: string) => edit({ mode: mode as Sound["mode"] });
	async function remove(sound: Sound) {
		if (!(await ask(`Remove "${sound.name}" from the soundboard? Keys that play it will stop working.`, { title: "Remove sound", kind: "warning" }))) return;
		editing = null;
		await invoke("sb_remove", { id: sound.id });
		refreshSoundboard();
	}

	let query = "";
	$: shown = (state?.sounds ?? []).filter((s) => !query.trim() || s.name.toLowerCase().includes(query.toLowerCase().trim()));
	const seconds = (ms: number) =>
		ms < 60000 ? `${(ms / 1000).toFixed(ms < 10000 ? 1 : 0)} s` : `${Math.floor(ms / 60000)}:${String(Math.round((ms % 60000) / 1000)).padStart(2, "0")}`;
	const MODE_LABEL = { overlap: "Play over", restart: "Restart", loop: "Loop" };
</script>

<svelte:window on:keydown={(e) => e.key == "Escape" && (editing = null)} />

<!-- svelte-ignore a11y-no-static-element-interactions -->
<div
	class="relative flex flex-col grow min-h-0 overflow-y-auto"
	on:dragenter={onDragEnter}
	on:dragleave={() => (dragOver = Math.max(0, dragOver - 1))}
	on:dragover={onDragOver}
	on:drop={onDrop}
>
	<div class="flex flex-col gap-4 w-full max-w-[1400px] mx-auto p-5">
		{#if state && !ready}
			<section class="flex flex-wrap items-center gap-4 p-5 rounded-2xl bg-neutral-900 border" style="border-color: var(--orvo-accent)">
				<div class="flex items-center justify-center w-12 h-12 rounded-xl shrink-0" style="background: color-mix(in srgb, var(--orvo-accent) 22%, transparent)">
					<Microphone size="26" class="text-neutral-100" />
				</div>
				<div class="grow min-w-[16rem]">
					<h2 class="font-semibold text-neutral-100">Set up the virtual mic (one time)</h2>
					<p class="mt-1 text-sm text-neutral-400 max-w-2xl">
						Orvo sends your voice and your sounds into a virtual mic that Discord, OBS and games can pick. It uses the free <strong class="text-neutral-200"
							>VB-Audio Virtual Cable</strong
						>: download it, run
						<em>VBCABLE_Setup_x64</em> as administrator, press <em>Install Driver</em>, then restart your PC. Until then, sounds still play in your headphones.
					</p>
				</div>
				<div class="flex gap-2 shrink-0">
					<button
						class="px-4 h-10 rounded-lg text-sm font-semibold text-white"
						style="background: var(--orvo-accent)"
						on:click={() => invoke("open_url", { url: state?.cable_url })}>Download VB-Cable</button
					>
					<button
						class="flex items-center gap-1.5 px-4 h-10 rounded-lg text-sm text-neutral-200 bg-neutral-800 border border-neutral-600 hover:bg-neutral-700"
						on:click={() => invoke("sb_refresh").then(() => setTimeout(refreshSoundboard, 500))}
					>
						<ArrowClockwise size="16" /> Check again
					</button>
				</div>
			</section>
		{/if}

		{#if state && config}
			<section class="grid gap-3 grid-cols-1 lg:grid-cols-3">
				<!-- Virtual mic -->
				<div class="flex flex-col gap-3 p-4 rounded-2xl bg-neutral-900 border border-neutral-700">
					<div class="flex items-center gap-2">
						<span class="w-2.5 h-2.5 rounded-full {ready && config.enabled && state.status.running ? 'bg-emerald-400' : 'bg-neutral-500'}"></span>
						<p class="orvo-label grow">Virtual mic</p>
						<label class="flex items-center gap-2 text-sm text-neutral-300 cursor-pointer">
							<input
								type="checkbox"
								class="accent-[var(--orvo-accent)] w-4 h-4"
								checked={config.enabled}
								on:change={(e) => setConfig({ enabled: e.currentTarget.checked })}
							/>
							On
						</label>
					</div>
					{#if ready}
						<p class="text-sm text-neutral-300">In Discord, OBS or a game, choose the mic <strong class="text-neutral-100">"CABLE Output"</strong>.</p>
					{:else}
						<p class="text-sm text-neutral-400">Not set up yet: see above.</p>
					{/if}
					<div class="flex flex-col gap-1">
						<div class="flex justify-between text-xs text-neutral-400"><span>Sounds in the mic</span><span>{pct(config.sound_gain)}%</span></div>
						<input
							type="range"
							min="0"
							max="2"
							step="0.01"
							value={config.sound_gain}
							aria-label="Sounds volume in the virtual mic"
							on:input={(e) => setConfig({ sound_gain: +e.currentTarget.value })}
						/>
					</div>
					{#if state.status.error}<p class="text-xs text-red-400">{state.status.error}</p>{/if}
				</div>

				<!-- Your mic -->
				<div class="flex flex-col gap-3 p-4 rounded-2xl bg-neutral-900 border border-neutral-700">
					<div class="flex items-center gap-2">
						<Microphone size="16" class="text-neutral-300" />
						<p class="orvo-label grow">Your mic</p>
						<label class="flex items-center gap-2 text-sm text-neutral-300 cursor-pointer" title="Send your voice into the virtual mic">
							<input
								type="checkbox"
								class="accent-[var(--orvo-accent)] w-4 h-4"
								checked={config.mic_on}
								on:change={(e) => setConfig({ mic_on: e.currentTarget.checked })}
							/>
							Send voice
						</label>
					</div>
					<div class="select-wrapper">
						<select
							class="w-full h-9"
							aria-label="Your mic"
							value={config.mic_device ?? ""}
							on:change={(e) => setConfig({ mic_device: e.currentTarget.value || null })}
						>
							<option value="">Windows default mic</option>
							{#each state.inputs as name}<option value={name}>{name}</option>{/each}
						</select>
					</div>
					<div class="flex flex-col gap-1">
						<div class="flex justify-between text-xs text-neutral-400"><span>Voice volume</span><span>{pct(config.mic_gain)}%</span></div>
						<input
							type="range"
							min="0"
							max="2"
							step="0.01"
							value={config.mic_gain}
							aria-label="Voice volume"
							on:input={(e) => setConfig({ mic_gain: +e.currentTarget.value })}
						/>
					</div>
				</div>

				<!-- Headphones -->
				<div class="flex flex-col gap-3 p-4 rounded-2xl bg-neutral-900 border border-neutral-700">
					<div class="flex items-center gap-2">
						<Headphones size="16" class="text-neutral-300" />
						<p class="orvo-label grow">Your headphones</p>
						<label class="flex items-center gap-2 text-sm text-neutral-300 cursor-pointer" title="Hear the sounds you play">
							<input
								type="checkbox"
								class="accent-[var(--orvo-accent)] w-4 h-4"
								checked={config.monitor_on}
								on:change={(e) => setConfig({ monitor_on: e.currentTarget.checked })}
							/>
							Hear sounds
						</label>
					</div>
					<div class="select-wrapper">
						<select
							class="w-full h-9"
							aria-label="Your headphones"
							value={config.monitor_device ?? ""}
							on:change={(e) => setConfig({ monitor_device: e.currentTarget.value || null })}
						>
							<option value="">Windows default output</option>
							{#each state.outputs as name}<option value={name}>{name}</option>{/each}
						</select>
					</div>
					<div class="flex flex-col gap-1">
						<div class="flex justify-between text-xs text-neutral-400"><span>Sounds volume</span><span>{pct(config.monitor_gain)}%</span></div>
						<input
							type="range"
							min="0"
							max="2"
							step="0.01"
							value={config.monitor_gain}
							aria-label="Sounds volume in your headphones"
							on:input={(e) => setConfig({ monitor_gain: +e.currentTarget.value })}
						/>
					</div>
				</div>
			</section>
		{/if}

		<!-- Sounds -->
		<section class="flex flex-col gap-3">
			<div class="flex flex-wrap items-center gap-2">
				<h2 class="text-lg font-semibold text-neutral-100">Sounds</h2>
				<span class="text-sm text-neutral-400">{state?.sounds.length ?? 0}</span>
				{#if adding > 0}<span class="text-sm animate-pulse" style="color: var(--orvo-accent)">Adding {adding}…</span>{/if}
				<div class="grow"></div>
				<label class="flex items-center h-9 px-3 gap-2 rounded-lg bg-neutral-800 border border-neutral-700">
					<MagnifyingGlass size="14" class="text-neutral-400" />
					<input bind:value={query} class="w-40 bg-transparent text-sm text-neutral-200" placeholder="Search sounds" type="search" spellcheck="false" />
				</label>
				<button
					class="flex items-center gap-1.5 px-3 h-9 rounded-lg text-sm text-neutral-200 bg-neutral-800 border border-neutral-600 hover:bg-neutral-700"
					on:click={stopAll}
				>
					<Stop size="16" weight="fill" /> Stop all
				</button>
				<button
					class="flex items-center gap-1.5 px-3 h-9 rounded-lg text-sm font-semibold text-white"
					style="background: var(--orvo-accent)"
					on:click={addFromDialog}
				>
					<Plus size="16" weight="bold" /> Add sounds
				</button>
			</div>

			{#if state && state.sounds.length == 0}
				<button
					class="flex flex-col items-center justify-center gap-3 h-72 rounded-2xl border-2 border-dashed border-neutral-600 hover:border-neutral-400 text-neutral-400 transition-colors"
					on:click={addFromDialog}
				>
					<UploadSimple size="40" />
					<span class="text-base text-neutral-200">Drag sound files here</span>
					<span class="text-sm">or click to choose them · mp3, wav, ogg, flac, m4a · whole folders work too</span>
				</button>
			{:else}
				<div class="grid gap-3" style="grid-template-columns: repeat(auto-fill, minmax(168px, 1fr))">
					{#each shown as sound (sound.id)}
						{@const on = playing.has(sound.id)}
						<div
							class="group relative flex flex-col rounded-2xl bg-neutral-900 border transition-colors {on ? '' : 'border-neutral-700 hover:border-neutral-500'}"
							style={on ? "border-color: var(--orvo-accent)" : ""}
						>
							<button class="flex flex-col items-start gap-2 p-3 pb-2 text-left" on:click={() => play(sound)} title="Play {sound.name}">
								<span
									class="flex items-center justify-center w-11 h-11 rounded-xl transition-colors"
									style={on ? "background: var(--orvo-accent)" : "background: color-mix(in srgb, var(--orvo-accent) 18%, #262626)"}
								>
									{#if on}<SpeakerHigh size="22" weight="fill" class="text-white" />{:else}<Play size="20" weight="fill" class="text-neutral-100" />{/if}
								</span>
								<span class="w-full text-sm font-semibold text-neutral-100 truncate">{sound.name}</span>
							</button>
							<div class="flex items-center gap-1.5 px-3 pb-3 text-xs text-neutral-400">
								<span>{seconds(sound.duration_ms)}</span>
								{#if sound.mode != "overlap"}<span class="px-1.5 rounded bg-neutral-800 text-neutral-300">{MODE_LABEL[sound.mode]}</span>{/if}
								{#if sound.volume != 1}<span class="px-1.5 rounded bg-neutral-800 text-neutral-300">{pct(sound.volume)}%</span>{/if}
							</div>
							<button
								class="absolute top-2 right-2 p-1.5 rounded-lg text-neutral-400 hover:text-neutral-100 hover:bg-neutral-800 opacity-0 group-hover:opacity-100 focus:opacity-100 transition-opacity"
								aria-label="Edit {sound.name}"
								on:click={() => (editing = sound)}
							>
								<DotsThreeVertical size="18" weight="bold" />
							</button>
						</div>
					{:else}
						<p class="col-span-full text-sm text-neutral-400">No sounds match "{query}".</p>
					{/each}
				</div>
				<p class="text-xs text-neutral-500">
					Tip: drag more files anywhere on this page. To put a sound on a key, open the Soundboard tab in the actions panel under your keys.
				</p>
			{/if}
		</section>
	</div>

	{#if dragOver > 0}
		<div
			class="pointer-events-none absolute inset-3 flex items-center justify-center rounded-2xl border-2 border-dashed bg-neutral-900/85 z-20"
			style="border-color: var(--orvo-accent)"
		>
			<div class="flex flex-col items-center gap-2 text-neutral-100">
				<UploadSimple size="40" />
				<p class="text-lg font-semibold">Drop to add sounds</p>
			</div>
		</div>
	{/if}
</div>

{#if editing}
	<!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
	<div class="fixed inset-0 z-40 bg-black/50" on:click={() => (editing = null)}></div>
	<div
		role="dialog"
		aria-label="Edit sound"
		class="fixed left-1/2 top-1/2 -translate-x-1/2 -translate-y-1/2 z-50 w-[24rem] max-w-[calc(100%-2rem)] p-5 flex flex-col gap-4 rounded-2xl bg-neutral-900 border border-neutral-600 shadow-2xl"
	>
		<label class="flex flex-col gap-1.5 text-sm text-neutral-400">
			Name
			<input
				class="h-10 px-3 rounded-lg bg-neutral-800 border border-neutral-600 text-neutral-100"
				value={editing.name}
				maxlength="60"
				on:input={(e) => edit({ name: e.currentTarget.value })}
			/>
		</label>
		<div class="flex flex-col gap-1.5">
			<div class="flex justify-between text-sm text-neutral-400"><span>Volume</span><span>{pct(editing.volume)}%</span></div>
			<input
				type="range"
				min="0"
				max="2"
				step="0.01"
				value={editing.volume}
				aria-label="Sound volume"
				on:input={(e) => edit({ volume: +e.currentTarget.value })}
			/>
		</div>
		<div class="flex flex-col gap-1.5">
			<span class="text-sm text-neutral-400">When pressed again while playing</span>
			<div role="radiogroup" aria-label="Play mode" class="grid grid-cols-3 gap-1 p-1 rounded-xl bg-neutral-800">
				{#each Object.entries(MODE_LABEL) as [mode, label]}
					<button
						role="radio"
						aria-checked={editing.mode == mode}
						class="h-9 rounded-lg text-sm transition-colors {editing.mode == mode
							? 'bg-neutral-950 text-neutral-100'
							: 'text-neutral-300 hover:text-neutral-100'}"
						on:click={() => setMode(mode)}>{label}</button
					>
				{/each}
			</div>
			<p class="text-xs text-neutral-500">
				{editing.mode == "overlap"
					? "Plays again on top of itself."
					: editing.mode == "restart"
						? "Starts again from the beginning."
						: "Repeats until you press it again."}
			</p>
		</div>
		<div class="flex items-center gap-2">
			<button class="flex items-center gap-1.5 px-3 h-9 rounded-lg text-sm text-red-300 hover:bg-red-950/40" on:click={() => editing && remove(editing)}
				><Trash size="16" /> Remove</button
			>
			<div class="grow"></div>
			<button
				class="px-3 h-9 rounded-lg text-sm text-neutral-200 bg-neutral-800 border border-neutral-600 hover:bg-neutral-700"
				on:click={() => editing && play(editing)}>Test</button
			>
			<button class="px-4 h-9 rounded-lg text-sm font-semibold text-white" style="background: var(--orvo-accent)" on:click={() => (editing = null)}>Done</button
			>
		</div>
	</div>
{/if}
