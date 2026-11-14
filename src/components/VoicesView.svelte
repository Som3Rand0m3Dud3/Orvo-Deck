<script lang="ts">
	// Orvo Voices: the voice changer as big picture tiles. One voice at a time; while a voice is
	// on, the manual voice effects in the Mixer are paused (and editing them turns the voice off).
	import ArrowCounterClockwise from "phosphor-svelte/lib/ArrowCounterClockwise";
	import FloppyDisk from "phosphor-svelte/lib/FloppyDisk";
	import Headphones from "phosphor-svelte/lib/Headphones";
	import SlidersHorizontal from "phosphor-svelte/lib/SlidersHorizontal";
	import Trash from "phosphor-svelte/lib/Trash";
	import X from "phosphor-svelte/lib/X";

	import { appView, refreshSoundboard, soundboard, voiceIcon, type VoiceTweak } from "$lib/soundboard";

	import { invoke } from "@tauri-apps/api/core";
	import { onDestroy, onMount } from "svelte";

	onMount(() => refreshSoundboard());

	$: state = $soundboard;
	$: config = state?.config;

	type Tile = { id: string; name: string; hint: string; mine: boolean };
	$: tiles = [
		...(state?.voice_presets ?? []).map((p): Tile => ({ id: p.id, name: p.name, hint: p.hint, mine: false })),
		...(config?.my_voices ?? []).map((v): Tile => ({ id: `my:${v.id}`, name: v.name, hint: "Your saved voice", mine: true })),
	];
	$: current = config?.voice_preset && config.voice_preset != "custom" ? config.voice_preset : "off";
	$: manualOn = (config?.voice_fx ?? []).some((e) => e.on);
	const iconUrl = (id: string, name: string) => voiceIcon(id, name).replace(/^opendeck\//, "/");

	async function pick(id: string) {
		await invoke("sb_set_voice", { id: current == id ? "off" : id });
		refreshSoundboard();
	}

	// hear yourself
	let listening = false;
	onDestroy(() => {
		if (listening) invoke("sb_listen", { on: false });
	});
	function toggleListen() {
		listening = !listening;
		invoke("sb_listen", { on: listening });
	}

	// tweak a voice
	let tweaking: Tile | null = null;
	const DEFAULT_TWEAK: VoiceTweak = { pitch: 0, amount: 1 };
	$: tweak = (tweaking && config?.voice_tweaks?.[tweaking.id]) || DEFAULT_TWEAK;
	async function openTweak(t: Tile) {
		tweaking = t;
		naming = false;
		if (current != t.id) await pick(t.id);
	}
	let tweakTimer: ReturnType<typeof setTimeout> | null = null;
	function setTweak(next: VoiceTweak) {
		if (!tweaking) return;
		const id = tweaking.id;
		soundboard.update((s) => (s ? { ...s, config: { ...s.config, voice_tweaks: { ...(s.config.voice_tweaks ?? {}), [id]: next } } } : s));
		if (tweakTimer) clearTimeout(tweakTimer);
		tweakTimer = setTimeout(() => invoke("sb_set_tweak", { id, pitch: next.pitch, amount: next.amount }), 120);
	}
	const num = (e: Event) => Number((e.currentTarget as HTMLInputElement).value);
	const pitchText = (v: number) => (v == 0 ? "No change" : `${v > 0 ? "+" : "−"}${Math.abs(v)} st · ${v > 0 ? "higher" : "deeper"}`);

	// save as my voice (the current voice with its tweak baked in)
	let naming = false;
	let newName = "";
	async function saveVoice() {
		if (!tweaking) return;
		if (current != tweaking.id) await invoke("sb_set_voice", { id: tweaking.id });
		await invoke("sb_save_voice", { name: newName || `My ${tweaking.name.toLowerCase()}` });
		naming = false;
		newName = "";
		tweaking = null;
		refreshSoundboard();
	}
	async function remove(t: Tile) {
		if (tweaking?.id == t.id) tweaking = null;
		await invoke("sb_delete_voice", { id: t.id.slice(3) });
		refreshSoundboard();
	}
</script>

<svelte:window on:keydown={(e) => e.key == "Escape" && ((naming = false), (tweaking = null))} />

<div class="flex flex-col grow min-h-0 gap-5 p-5 overflow-auto">
	{#if !state || !config}
		<p class="text-neutral-400">Loading voices…</p>
	{:else}
		<div class="flex flex-wrap items-end gap-3">
			<div class="grow">
				<h1 class="text-xl font-bold text-neutral-100">Voices</h1>
				<p class="text-sm text-neutral-400">
					Click a voice to turn it on, click it again to turn it off. While a voice is on, your own effects in the Mixer are paused.
				</p>
			</div>
			<button
				class="flex items-center gap-1.5 px-3.5 h-10 rounded-lg text-sm border {listening
					? 'text-white border-transparent'
					: 'text-neutral-200 bg-neutral-800 border-neutral-600 hover:bg-neutral-700'}"
				style={listening ? "background: var(--orvo-accent)" : ""}
				aria-pressed={listening}
				title="Hear your voice in your headphones"
				on:click={toggleListen}><Headphones size="16" /> {listening ? "Listening…" : "Hear it"}</button
			>
		</div>

		{#if !config.mic_on}
			<p class="px-3.5 py-2.5 rounded-xl border border-amber-700/60 bg-amber-950/30 text-sm text-amber-200">
				Your voice is muted, so others won't hear any voice. Unmute “Your voice” in the Mixer.
			</p>
		{/if}

		<div role="radiogroup" aria-label="Voice changer" class="grid gap-3" style="grid-template-columns: repeat(auto-fill, minmax(9.5rem, 1fr))">
			{#each tiles as t (t.id)}
				{@const on = current == t.id}
				<div class="relative group">
					<button
						role="radio"
						aria-checked={on}
						title={t.hint}
						class="w-full flex flex-col items-center gap-2 pt-5 pb-3.5 px-2 rounded-2xl border-2 transition-colors {on
							? 'bg-[#1D2A44]'
							: 'bg-neutral-900 border-neutral-700 hover:border-neutral-500'}"
						style={on ? "border-color: var(--orvo-accent)" : ""}
						on:click={() => pick(t.id)}
					>
						<img src={iconUrl(t.id, t.name)} alt="" class="w-20 h-20 rounded-2xl pointer-events-none" />
						<span class="text-base font-semibold text-neutral-100">{t.name}</span>
						<span class="text-xs text-neutral-400 leading-snug text-center min-h-[2lh]"
							>{t.id == "off" ? (manualOn ? "Your Mixer effects" : "Your normal voice") : t.hint}</span
						>
						{#if on}
							<span class="absolute top-2 left-2 px-2 py-0.5 rounded-full text-[10px] font-bold tracking-wider text-white" style="background: var(--orvo-accent)"
								>ON</span
							>
						{/if}
					</button>
					{#if t.id != "off"}
						<div class="absolute top-2 right-2 flex gap-1 {on || tweaking?.id == t.id ? '' : 'opacity-0 group-hover:opacity-100 focus-within:opacity-100'}">
							{#if t.mine}
								<button
									class="p-1.5 rounded-lg bg-neutral-800/90 text-neutral-400 hover:text-red-300"
									aria-label="Delete {t.name}"
									title="Delete"
									on:click={() => remove(t)}><Trash size="14" /></button
								>
							{/if}
							<button
								class="p-1.5 rounded-lg bg-neutral-800/90 text-neutral-300 hover:text-neutral-100"
								aria-label="Tweak {t.name}"
								title="Tweak"
								on:click={() => openTweak(t)}><SlidersHorizontal size="14" /></button
							>
						</div>
					{/if}
				</div>
			{/each}
		</div>

		<p class="text-xs text-neutral-500">
			Put voices on pad keys from the actions panel's Voices tab (on the
			<button class="underline" on:click={() => appView.set("pad")}>Orvo M1</button> page). Voice cleanup (noise and breaths) always stays on.
		</p>

		{#if tweaking}
			<!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
			<div class="fixed inset-0 z-40 bg-black/50" on:click={() => (tweaking = null)}></div>
			<div
				role="dialog"
				aria-label="Tweak {tweaking.name}"
				class="fixed left-1/2 top-1/2 -translate-x-1/2 -translate-y-1/2 z-50 w-[26rem] p-5 flex flex-col gap-4 rounded-2xl bg-neutral-900 border border-neutral-600 shadow-2xl"
			>
				<div class="flex items-center gap-3">
					<img src={iconUrl(tweaking.id, tweaking.name)} alt="" class="w-12 h-12 rounded-xl" />
					<div class="grow">
						<h2 class="text-lg font-bold text-neutral-100">Tweak {tweaking.name}</h2>
						<p class="text-xs text-neutral-400">Changes are live. Turn on “Hear it” to listen.</p>
					</div>
					<button class="p-1.5 rounded-lg text-neutral-400 hover:text-neutral-100" aria-label="Close" on:click={() => (tweaking = null)}><X size="18" /></button>
				</div>
				<label class="flex flex-col gap-1.5">
					<span class="flex justify-between text-sm text-neutral-300">Pitch <span class="text-neutral-400">{pitchText(tweak.pitch)}</span></span>
					<input
						type="range"
						min="-12"
						max="12"
						step="0.5"
						value={tweak.pitch}
						on:input={(e) => setTweak({ ...tweak, pitch: num(e) })}
						on:dblclick={() => setTweak({ ...tweak, pitch: 0 })}
						style="accent-color: var(--orvo-accent)"
					/>
				</label>
				<label class="flex flex-col gap-1.5">
					<span class="flex justify-between text-sm text-neutral-300">Strength <span class="text-neutral-400">{Math.round(tweak.amount * 100)}%</span></span>
					<input
						type="range"
						min="0"
						max="1"
						step="0.01"
						value={tweak.amount}
						on:input={(e) => setTweak({ ...tweak, amount: num(e) })}
						on:dblclick={() => setTweak({ ...tweak, amount: 1 })}
						style="accent-color: var(--orvo-accent)"
					/>
				</label>
				{#if naming}
					<form class="flex gap-2" on:submit|preventDefault={saveVoice}>
						<!-- svelte-ignore a11y-autofocus -->
						<input
							bind:value={newName}
							autofocus
							maxlength="40"
							aria-label="Name for the new voice"
							placeholder="My {tweaking.name.toLowerCase()}"
							class="grow h-9 px-3 rounded-lg bg-neutral-800 border border-neutral-600 text-sm text-neutral-100"
						/>
						<button type="submit" class="px-4 h-9 rounded-lg text-sm font-semibold text-white" style="background: var(--orvo-accent)">Save</button>
					</form>
				{:else}
					<div class="flex items-center gap-2">
						<button
							class="flex items-center gap-1.5 px-3 h-9 rounded-lg text-sm text-neutral-300 hover:bg-neutral-800 disabled:opacity-40"
							disabled={tweak.pitch == 0 && tweak.amount == 1}
							on:click={() => setTweak(DEFAULT_TWEAK)}><ArrowCounterClockwise size="15" /> Reset</button
						>
						<div class="grow"></div>
						<button
							class="flex items-center gap-1.5 px-3.5 h-9 rounded-lg text-sm font-semibold text-white"
							style="background: var(--orvo-accent)"
							on:click={() => (naming = true)}><FloppyDisk size="15" /> Save as my voice</button
						>
					</div>
				{/if}
			</div>
		{/if}
	{/if}
</div>
