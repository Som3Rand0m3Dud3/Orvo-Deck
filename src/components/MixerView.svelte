<script lang="ts">
	// Orvo Mixer: three channel strips (your voice, the soundboard, the virtual mic) and the
	// effects of the selected one. Voice effects run after Voice cleanup; the virtual mic's
	// limiter catches anything that got too loud.
	import ArrowRight from "phosphor-svelte/lib/ArrowRight";
	import DotsSixVertical from "phosphor-svelte/lib/DotsSixVertical";
	import FloppyDisk from "phosphor-svelte/lib/FloppyDisk";
	import Headphones from "phosphor-svelte/lib/Headphones";
	import Microphone from "phosphor-svelte/lib/Microphone";
	import Plus from "phosphor-svelte/lib/Plus";
	import SpeakerHigh from "phosphor-svelte/lib/SpeakerHigh";
	import X from "phosphor-svelte/lib/X";

	import {
		EFFECTS,
		MIC_PRESETS,
		appView,
		formatParam,
		newEffect,
		paramValue,
		refreshSoundboard,
		soundboard,
		updateConfig,
		voiceIcon,
		type EffectSpec,
		type SoundboardConfig,
	} from "$lib/soundboard";

	import { invoke } from "@tauri-apps/api/core";
	import { onDestroy, onMount } from "svelte";

	type StripId = "voice" | "sound" | "out";
	let selected: StripId = "voice";
	try {
		const s = localStorage.getItem("orvo-mixer-strip");
		if (s == "voice" || s == "sound" || s == "out") selected = s;
	} catch {}
	$: try {
		localStorage.setItem("orvo-mixer-strip", selected);
	} catch {}

	$: state = $soundboard;
	$: config = state?.config;

	onMount(() => refreshSoundboard());

	// meters
	type Meter = { out_db: number; sound_db: number; mix_db: number; running: boolean };
	let meter: Meter = { out_db: -120, sound_db: -120, mix_db: -120, running: false };
	const timer = setInterval(async () => {
		meter = await invoke<Meter>("sb_meter").catch(() => meter);
	}, 60);
	let listening = false;
	onDestroy(() => {
		clearInterval(timer);
		if (listening) invoke("sb_listen", { on: false });
	});
	function toggleListen() {
		listening = !listening;
		invoke("sb_listen", { on: listening });
	}
	const SEGS = 28;
	const lit = (dbv: number) => Math.round(Math.max(0, Math.min(1, (dbv + 60) / 60)) * SEGS);

	// strips
	const FX_KEY: { [k in StripId]: "voice_fx" | "sound_fx" | "out_fx" } = { voice: "voice_fx", sound: "sound_fx", out: "out_fx" };
	$: strips = config
		? [
				{
					id: "voice" as StripId,
					name: "Your voice",
					sub: config.mic_preset == "custom" ? "Custom cleanup" : `${MIC_PRESETS[config.mic_preset]?.label ?? "Studio"} cleanup`,
					gainKey: "mic_gain" as const,
					gain: config.mic_gain,
					muted: !config.mic_on,
					db: meter.out_db,
					fx: config.voice_fx ?? [],
				},
				{
					id: "sound" as StripId,
					name: "Soundboard",
					sub: "All sounds",
					gainKey: "master_gain" as const,
					gain: config.master_gain ?? 1,
					muted: config.sound_mute,
					db: meter.sound_db,
					fx: config.sound_fx ?? [],
				},
				{
					id: "out" as StripId,
					name: "Virtual mic",
					sub: "What others hear",
					gainKey: "out_gain" as const,
					gain: config.out_gain ?? 1,
					muted: config.out_mute,
					db: meter.mix_db,
					fx: config.out_fx ?? [],
				},
			]
		: [];
	function toggleMute(id: StripId) {
		if (!config) return;
		if (id == "voice") updateConfig({ mic_on: !config.mic_on });
		else if (id == "sound") updateConfig({ sound_mute: !config.sound_mute });
		else updateConfig({ out_mute: !config.out_mute });
	}

	// effects of the selected strip
	$: chain = config ? ((config[FX_KEY[selected]] as EffectSpec[]) ?? []) : [];
	function setChain(next: EffectSpec[]) {
		const patch: Partial<SoundboardConfig> = { [FX_KEY[selected]]: next };
		// your own effects and the voice changer don't mix: editing them turns the voice off
		if (selected == "voice" && config?.voice_preset != "off") patch.voice_preset = "off";
		updateConfig(patch);
	}
	const setEffect = (i: number, e: EffectSpec) => setChain(chain.map((x, n) => (n == i ? e : x)));
	const setParam = (i: number, key: string, v: number) => setEffect(i, { ...chain[i], params: { ...chain[i].params, [key]: v } });

	let adding = false;
	function addEffect(kind: string) {
		adding = false;
		setChain([...chain, newEffect(kind)]);
	}

	// drag cards to reorder
	let dragFrom: number | null = null;
	let dragOver: number | null = null;
	function drop(to: number) {
		if (dragFrom == null || dragFrom == to) return (dragFrom = dragOver = null);
		const next = [...chain];
		const [moved] = next.splice(dragFrom, 1);
		next.splice(to, 0, moved);
		dragFrom = dragOver = null;
		setChain(next);
	}

	// voice presets
	$: presets = [
		...(state?.voice_presets ?? []).map((p) => ({ id: p.id, name: p.name, hint: p.hint, mine: false })),
		...(config?.my_voices ?? []).map((v) => ({ id: `my:${v.id}`, name: v.name, hint: "Your saved voice", mine: true })),
		...(config?.ai_voices ?? []).map((v) => ({ id: `ai:${v.id}`, name: v.name, hint: "AI voice", mine: true, image: v.image })),
	];
	const iconUrl = (id: string, name: string, image?: string | null) => voiceIcon(id, name, image).replace(/^opendeck\//, "/");
	const imageOf = (p: object) => ("image" in p ? (p.image as string | null) : null);
	$: activeVoice = config?.voice_preset && config.voice_preset != "off" ? presets.find((p) => p.id == config!.voice_preset) : undefined;
	async function pickVoice(id: string) {
		await invoke("sb_set_voice", { id });
		refreshSoundboard();
	}
	let naming = false;
	let newName = "";
	async function saveVoice() {
		naming = false;
		await invoke("sb_save_voice", { name: newName });
		newName = "";
		refreshSoundboard();
	}

	$: stripName = strips.find((s) => s.id == selected)?.name ?? "";
	const pct = (v: number) => Math.round(v * 100);
</script>

<svelte:window on:keydown={(e) => e.key == "Escape" && ((adding = false), (naming = false))} />

<div class="flex grow min-h-0 gap-5 p-5 overflow-auto">
	{#if !state || !config}
		<p class="text-neutral-400">Loading the mixer…</p>
	{:else}
		<div class="flex flex-col gap-3 shrink-0">
			<div class="flex items-baseline gap-2">
				<h1 class="text-xl font-bold text-neutral-100">Mixer</h1>
				<span class="text-sm text-neutral-400">· goes to “CABLE Output”</span>
			</div>
			{#if !state.cable_installed}
				<p class="max-w-[34rem] text-sm text-amber-300">The virtual mic isn't set up yet: open the Soundboard tab for the one-time setup.</p>
			{/if}
			<div class="flex gap-3">
				{#each strips as strip (strip.id)}
					{@const on = selected == strip.id}
					<section
						aria-label="{strip.name} channel"
						class="flex flex-col gap-3 w-44 p-3.5 rounded-2xl bg-neutral-900 border transition-colors {on ? '' : 'border-neutral-700 hover:border-neutral-500'}"
						style={on ? "border-color: var(--orvo-accent); border-width: 2px" : ""}
					>
						<button class="flex items-center gap-2 text-left" on:click={() => (selected = strip.id)}>
							{#if strip.id == "voice"}<Microphone size="18" class="text-neutral-300" />{:else if strip.id == "sound"}<SpeakerHigh
									size="18"
									class="text-neutral-300"
								/>{:else}<ArrowRight size="18" class="text-neutral-300" />{/if}
							<span class="flex flex-col">
								<span class="text-sm font-semibold text-neutral-100">{strip.name}</span>
								<span class="text-[11px] text-neutral-400">{strip.sub}</span>
							</span>
						</button>
						<div class="flex gap-1.5">
							<button
								class="grow h-8 rounded-lg border text-xs font-semibold {strip.muted
									? 'bg-red-950/60 border-red-800 text-red-300'
									: 'bg-neutral-800 border-neutral-600 text-neutral-300'}"
								aria-pressed={strip.muted}
								title={strip.muted ? "Muted: click to unmute" : "Mute"}
								on:click={() => toggleMute(strip.id)}>M</button
							>
							{#if strip.id == "voice"}
								<button
									class="grow h-8 flex items-center justify-center rounded-lg border {listening
										? 'text-white border-transparent'
										: 'bg-neutral-800 border-neutral-600 text-neutral-300'}"
									style={listening ? "background: var(--orvo-accent)" : ""}
									aria-pressed={listening}
									aria-label="Hear my voice"
									title="Hear your voice (with its effects) in your headphones"
									on:click={toggleListen}><Headphones size="15" /></button
								>
							{:else if strip.id == "sound"}
								<button
									class="grow h-8 flex items-center justify-center rounded-lg border {config.monitor_on
										? 'text-white border-transparent'
										: 'bg-neutral-800 border-neutral-600 text-neutral-300'}"
									style={config.monitor_on ? "background: var(--orvo-accent)" : ""}
									aria-pressed={config.monitor_on}
									aria-label="Hear the sounds"
									title="Hear the sounds in your headphones"
									on:click={() => updateConfig({ monitor_on: !config?.monitor_on })}><Headphones size="15" /></button
								>
							{/if}
						</div>
						<div class="flex justify-center gap-3 h-72 py-1.5">
							<div class="flex flex-col-reverse gap-[3px] w-2" aria-hidden="true">
								{#each Array(SEGS) as _, i}
									<div class="grow rounded-sm" style="background: {i < lit(strip.db) ? (i >= SEGS - 4 ? '#F59E0B' : 'var(--orvo-accent)') : '#262B34'}"></div>
								{/each}
							</div>
							<input
								type="range"
								min="0"
								max="2"
								step="0.01"
								value={strip.gain}
								aria-label="{strip.name} volume"
								class="fader"
								on:input={(e) => updateConfig({ [strip.gainKey]: +e.currentTarget.value })}
								on:dblclick={() => updateConfig({ [strip.gainKey]: 1 })}
							/>
							<div class="w-2"></div>
						</div>
						<div class="text-center text-xl font-bold tabular-nums text-neutral-100">{pct(strip.gain)}%</div>
						<div class="flex flex-col gap-1.5">
							<span class="text-[11px] font-semibold tracking-wider uppercase text-neutral-400">Effects</span>
							{#each strip.fx as e (e.id)}
								<button
									class="flex items-center gap-1.5 px-2 py-1.5 rounded-lg border text-xs text-left {e.on
										? 'bg-[#22304A] border-[#3B5A95] text-neutral-100'
										: 'bg-neutral-800 border-neutral-600 text-neutral-500'}"
									on:click={() => (selected = strip.id)}
								>
									<span class="w-1.5 h-1.5 rounded-full shrink-0" style="background: {e.on ? 'var(--orvo-accent)' : '#4B5262'}"></span>{EFFECTS[e.kind]
										?.label ?? e.kind}
								</button>
							{:else}
								<span class="text-xs text-neutral-500">None</span>
							{/each}
							<button
								class="flex items-center justify-center gap-1.5 h-8 rounded-lg border border-dashed border-neutral-600 text-xs text-neutral-400 hover:text-neutral-100"
								on:click={() => {
									selected = strip.id;
									adding = true;
								}}><Plus size="13" /> Add effect</button
							>
						</div>
					</section>
				{/each}
			</div>
			<p class="text-xs text-neutral-500">Double-click a fader to reset it to 100%.</p>
		</div>

		<section aria-label="Effects" class="relative flex flex-col gap-4 grow min-w-[30rem] p-5 rounded-2xl bg-neutral-900 border border-neutral-700">
			<div class="flex flex-wrap items-center gap-2">
				<div class="grow min-w-[14rem]">
					<h2 class="text-lg font-bold text-neutral-100">{stripName} · effects</h2>
					<p class="text-sm text-neutral-400">
						{selected == "voice"
							? "Runs after Voice cleanup. Drag cards to change the order."
							: selected == "sound"
								? "On every sound together, and in your headphones too. Single sounds can have their own effects in their ⋮ window."
								: "On everything others hear: your voice and the sounds together."}
					</p>
				</div>
				{#if selected == "voice"}
					<button
						class="flex items-center gap-1.5 px-3 h-9 rounded-lg text-sm border {listening
							? 'text-white border-transparent'
							: 'text-neutral-200 bg-neutral-800 border-neutral-600 hover:bg-neutral-700'}"
						style={listening ? "background: var(--orvo-accent)" : ""}
						aria-pressed={listening}
						on:click={toggleListen}><Headphones size="15" /> {listening ? "Listening…" : "Hear it"}</button
					>
					<button
						class="flex items-center gap-1.5 px-3 h-9 rounded-lg text-sm text-neutral-200 bg-neutral-800 border border-neutral-600 hover:bg-neutral-700"
						on:click={() => (naming = true)}><FloppyDisk size="15" /> Save as my voice</button
					>
				{/if}
				<button
					class="flex items-center gap-1.5 px-3.5 h-9 rounded-lg text-sm font-semibold text-white"
					style="background: var(--orvo-accent)"
					on:click={() => (adding = !adding)}><Plus size="15" weight="bold" /> Add effect</button
				>
			</div>

			{#if selected == "voice"}
				{#if activeVoice}
					<div
						role="status"
						class="flex flex-wrap items-center gap-3 px-3.5 py-2.5 rounded-xl border bg-[#1D2A44] text-sm text-neutral-100"
						style="border-color: var(--orvo-accent)"
					>
						<img src={iconUrl(activeVoice.id, activeVoice.name, imageOf(activeVoice))} alt="" class="w-8 h-8 rounded-lg" />
						<span class="grow">Voice changer on: <strong>{activeVoice.name}</strong>. <span class="text-neutral-300">Your effects below are paused.</span></span>
						<button class="underline" style="color: #7FA8FF" on:click={() => appView.set("voices")}>Voices</button>
						<button class="px-3 h-8 rounded-lg text-sm text-neutral-100 bg-neutral-800 border border-neutral-600 hover:bg-neutral-700" on:click={() => pickVoice("off")}
							>Turn off</button
						>
					</div>
				{/if}
				<div class="flex items-center gap-2.5 px-3.5 py-2.5 rounded-xl bg-neutral-950/60 border border-neutral-700 text-sm text-neutral-300">
					<Microphone size="15" class="text-neutral-400" />
					<span class="grow"
						>Voice cleanup · <strong class="text-neutral-100"
							>{config.mic_preset == "custom" ? "Custom" : (MIC_PRESETS[config.mic_preset]?.label ?? "Studio")}</strong
						>
						<span class="text-neutral-500">(noise, breaths, gate, EQ, compressor) always runs first</span></span
					>
					<button class="underline" style="color: #7FA8FF" on:click={() => appView.set("soundboard")}>Adjust</button>
				</div>
			{/if}

			<div class="flex flex-col gap-3 transition-opacity" class:opacity-50={selected == "voice" && !!activeVoice}>
				{#each chain as e, i (e.id)}
					{@const def = EFFECTS[e.kind]}
					<!-- svelte-ignore a11y-no-static-element-interactions -->
					<div
						class="flex flex-col gap-3 px-4 py-3.5 rounded-xl bg-[#1C1F26] border transition-colors {dragOver == i && dragFrom != i
							? 'border-dashed'
							: 'border-neutral-700'}"
						style={dragOver == i && dragFrom != i ? "border-color: var(--orvo-accent)" : ""}
						class:opacity-60={!e.on}
						on:dragover|preventDefault={() => (dragOver = i)}
						on:drop|preventDefault={() => drop(i)}
					>
						<div class="flex items-center gap-2.5">
							<span
								class="text-neutral-500 cursor-grab active:cursor-grabbing"
								draggable="true"
								title="Drag to reorder"
								role="button"
								tabindex="-1"
								aria-label="Drag {def?.label ?? e.kind} to reorder"
								on:dragstart={(ev) => {
									dragFrom = i;
									ev.dataTransfer?.setData("text/plain", e.id);
								}}
								on:dragend={() => (dragFrom = dragOver = null)}><DotsSixVertical size="18" /></span
							>
							<span class="grow text-sm font-semibold text-neutral-100"
								>{def?.label ?? e.kind} <span class="font-normal text-xs text-neutral-400">· {def?.desc ?? ""}</span></span
							>
							<label class="flex items-center gap-1.5 text-xs text-neutral-400 cursor-pointer">
								<input
									type="checkbox"
									class="accent-[var(--orvo-accent)] w-4 h-4"
									checked={e.on}
									on:change={(ev) => setEffect(i, { ...e, on: ev.currentTarget.checked })}
								/>
								On
							</label>
							<button
								class="p-1.5 rounded-lg text-neutral-500 hover:text-neutral-100 hover:bg-neutral-800"
								aria-label="Remove {def?.label ?? e.kind}"
								on:click={() => setChain(chain.filter((_, n) => n != i))}><X size="14" /></button
							>
						</div>
						{#if def}
							<div class="grid gap-x-5 gap-y-3 grid-cols-2 xl:grid-cols-3">
								{#each def.params as p}
									{@const v = paramValue(e, p)}
									{#if p.unit == "toggle"}
										<label class="flex items-center justify-between gap-2 text-sm text-neutral-200">
											{p.label}
											<input
												type="checkbox"
												class="accent-[var(--orvo-accent)] w-4 h-4"
												checked={v >= 0.5}
												on:change={(ev) => setParam(i, p.key, ev.currentTarget.checked ? 1 : 0)}
											/>
										</label>
									{:else}
										<label class="flex flex-col gap-1 text-sm text-neutral-200">
											<span class="flex justify-between"
												><span>{p.label}</span><span class="text-xs text-neutral-400 tabular-nums">{formatParam(p, v)}</span></span
											>
											<input
												type="range"
												min={p.min}
												max={p.max}
												step={p.step}
												value={v}
												on:input={(ev) => setParam(i, p.key, +ev.currentTarget.value)}
												on:dblclick={() => setParam(i, p.key, p.def)}
											/>
										</label>
									{/if}
								{/each}
							</div>
						{/if}
					</div>
				{:else}
					<button
						class="flex flex-col items-center justify-center gap-2 h-40 rounded-xl border-2 border-dashed border-neutral-700 text-neutral-400 hover:border-neutral-500"
						on:click={() => (adding = true)}
					>
						<Plus size="28" />
						<span>No effects yet: add one{selected == "voice" ? " or pick a voice in the Voices tab" : ""}</span>
					</button>
				{/each}
			</div>

			{#if adding}
				<!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
				<div class="fixed inset-0 z-30" on:click={() => (adding = false)}></div>
				<div
					role="dialog"
					aria-label="Add effect"
					class="absolute top-16 right-5 z-40 w-[34rem] max-w-[calc(100%-2.5rem)] p-3.5 flex flex-col gap-2.5 rounded-xl bg-[#1A1D23] border border-neutral-600 shadow-2xl"
				>
					<div class="flex items-center">
						<span class="grow font-bold text-neutral-100">Add an effect</span><span class="text-xs text-neutral-400">to {stripName}</span>
					</div>
					<div class="grid grid-cols-2 gap-2">
						{#each Object.entries(EFFECTS) as [kind, def]}
							<button
								class="flex flex-col items-start gap-0.5 px-3 py-2.5 rounded-lg border border-neutral-700 bg-[#1C1F26] hover:border-neutral-500 text-left"
								on:click={() => addEffect(kind)}
							>
								<span class="text-sm font-semibold text-neutral-100">{def.label}</span>
								<span class="text-xs text-neutral-400">{def.desc}</span>
							</button>
						{/each}
					</div>
				</div>
			{/if}

			{#if naming}
				<!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
				<div class="fixed inset-0 z-40 bg-black/50" on:click={() => (naming = false)}></div>
				<form
					role="dialog"
					aria-label="Save voice"
					class="fixed left-1/2 top-1/2 -translate-x-1/2 -translate-y-1/2 z-50 w-[24rem] p-5 flex flex-col gap-3 rounded-2xl bg-neutral-900 border border-neutral-600 shadow-2xl"
					on:submit|preventDefault={saveVoice}
				>
					<label class="flex flex-col gap-1.5 text-sm text-neutral-400">
						Name this voice
						<!-- svelte-ignore a11y-autofocus -->
						<input
							bind:value={newName}
							autofocus
							maxlength="40"
							placeholder="My robot"
							class="h-10 px-3 rounded-lg bg-neutral-800 border border-neutral-600 text-neutral-100"
						/>
					</label>
					<p class="text-xs text-neutral-500">It appears in the Voices tab, and you can put it on a pad key from the actions panel's Voices tab.</p>
					<div class="flex justify-end gap-2">
						<button type="button" class="px-3 h-9 rounded-lg text-sm text-neutral-300 hover:bg-neutral-800" on:click={() => (naming = false)}>Cancel</button>
						<button type="submit" class="px-4 h-9 rounded-lg text-sm font-semibold text-white" style="background: var(--orvo-accent)">Save</button>
					</div>
				</form>
			{/if}
		</section>
	{/if}
</div>

<style>
	.fader {
		writing-mode: vertical-lr;
		direction: rtl;
		width: 28px;
		height: 100%;
		margin: 0;
		accent-color: var(--orvo-accent);
	}
</style>
