<script lang="ts">
	// Orvo Soundboard: trim a sound by dragging its start and end over the waveform.
	import Play from "phosphor-svelte/lib/Play";
	import Stop from "phosphor-svelte/lib/Stop";

	import type { Sound } from "$lib/soundboard";

	import { invoke } from "@tauri-apps/api/core";
	import { createEventDispatcher, onDestroy, onMount } from "svelte";

	export let sound: Sound;

	const dispatch = createEventDispatcher<{ change: { start_ms: number; end_ms: number | null; fade_out: boolean } }>();
	const POINTS = 480;
	const MIN_LEN = 20; // ms

	$: total = Math.max(1, sound.duration_ms);
	$: start = Math.min(sound.start_ms ?? 0, total);
	$: end = Math.min(sound.end_ms ?? total, total);

	let peaks: number[] = [];
	let failed = false;
	onMount(async () => {
		try {
			peaks = await invoke<number[]>("sb_waveform", { id: sound.id, points: POINTS });
		} catch {
			failed = true;
		}
	});

	let box: HTMLDivElement;
	let width = 0;
	const pct = (ms: number) => (ms / total) * 100;

	function emit(s: number, e: number, fade = sound.fade_out ?? true) {
		s = Math.round(Math.max(0, Math.min(s, e - MIN_LEN)));
		e = Math.round(Math.min(total, Math.max(e, s + MIN_LEN)));
		dispatch("change", { start_ms: s, end_ms: e >= total ? null : e, fade_out: fade });
	}

	// dragging: the handle nearest to where you pressed follows the pointer
	let dragging: "start" | "end" | null = null;
	function msAt(event: PointerEvent) {
		const r = box.getBoundingClientRect();
		return Math.max(0, Math.min(1, (event.clientX - r.left) / r.width)) * total;
	}
	function down(event: PointerEvent) {
		if (event.button != 0) return;
		const t = msAt(event);
		dragging = Math.abs(t - start) <= Math.abs(t - end) ? "start" : "end";
		box.setPointerCapture(event.pointerId);
		move(event);
	}
	function move(event: PointerEvent) {
		if (!dragging) return;
		const t = msAt(event);
		if (dragging == "start") emit(t, end);
		else emit(start, t);
	}
	function up() {
		dragging = null;
	}
	function key(which: "start" | "end", event: KeyboardEvent) {
		const step = event.shiftKey ? 100 : 10;
		const d = event.key == "ArrowLeft" ? -step : event.key == "ArrowRight" ? step : 0;
		if (!d) return;
		event.preventDefault();
		if (which == "start") emit(start + d, end);
		else emit(start, end + d);
	}

	// preview (your headphones only) with a moving line
	let playingFrom = 0;
	let playhead: number | null = null;
	let raf = 0;
	async function preview() {
		await invoke("sb_preview", { id: sound.id, startMs: start, endMs: end >= total ? null : end, fadeOut: sound.fade_out ?? true });
		playingFrom = performance.now();
		cancelAnimationFrame(raf);
		const tickFn = () => {
			const t = start + (performance.now() - playingFrom);
			if (t >= end) {
				playhead = null;
				return;
			}
			playhead = t;
			raf = requestAnimationFrame(tickFn);
		};
		tickFn();
	}
	function stopPreview() {
		invoke("sb_stop", { id: `preview:${sound.id}` });
		cancelAnimationFrame(raf);
		playhead = null;
	}
	onDestroy(() => {
		cancelAnimationFrame(raf);
		if (playhead != null) invoke("sb_stop", { id: `preview:${sound.id}` });
	});

	const secs = (ms: number) => `${(ms / 1000).toFixed(2)} s`;
	$: trimmed = start > 0 || end < total;
</script>

<div class="flex flex-col gap-2">
	<div class="flex items-center justify-between text-sm text-neutral-400">
		<span>Trim</span>
		<span class="text-xs">starts at {secs(start)} · length {secs(end - start)}{trimmed ? ` of ${secs(total)}` : ""}</span>
	</div>

	<!-- svelte-ignore a11y-no-static-element-interactions -->
	<div
		bind:this={box}
		bind:clientWidth={width}
		class="relative h-24 rounded-xl bg-neutral-950 border border-neutral-700 overflow-hidden select-none touch-none cursor-ew-resize"
		on:pointerdown={down}
		on:pointermove={move}
		on:pointerup={up}
		on:pointercancel={up}
	>
		{#if peaks.length}
			<svg class="absolute inset-0 w-full h-full" viewBox="0 0 {POINTS} 100" preserveAspectRatio="none" aria-hidden="true">
				{#each peaks as p, i}
					{@const h = Math.max(1.5, p * 92)}
					{@const inside = (i / POINTS) * total >= start && (i / POINTS) * total <= end}
					<rect x={i} y={50 - h / 2} width="0.7" height={h} fill={inside ? "var(--orvo-accent)" : "#525252"} />
				{/each}
			</svg>
		{:else}
			<p class="absolute inset-0 flex items-center justify-center text-xs text-neutral-500">{failed ? "Couldn't read this sound" : "Loading…"}</p>
		{/if}
		<!-- cut parts are dimmed -->
		<div class="absolute inset-y-0 left-0 bg-black/55 pointer-events-none" style="width: {pct(start)}%"></div>
		<div class="absolute inset-y-0 right-0 bg-black/55 pointer-events-none" style="width: {100 - pct(end)}%"></div>
		{#if playhead != null}<div class="absolute inset-y-0 w-0.5 bg-white pointer-events-none" style="left: {pct(playhead)}%"></div>{/if}
		<!-- handles -->
		{#each [["start", start], ["end", end]] as [which, at]}
			<div
				role="slider"
				tabindex="0"
				aria-label={which == "start" ? "Trim start" : "Trim end"}
				aria-valuemin={0}
				aria-valuemax={total}
				aria-valuenow={Math.round(Number(at))}
				aria-valuetext={secs(Number(at))}
				class="absolute inset-y-0 -ml-[7px] w-[14px] flex justify-center outline-none group"
				style="left: {pct(Number(at))}%"
				on:keydown={(e) => key(which == "start" ? "start" : "end", e)}
			>
				<div class="w-[3px] h-full rounded-full group-focus-visible:w-[5px]" style="background: var(--orvo-accent)"></div>
				<div class="absolute top-1/2 -translate-y-1/2 w-[14px] h-8 rounded-md border-2 bg-neutral-900" style="border-color: var(--orvo-accent)"></div>
			</div>
		{/each}
	</div>

	<div class="flex flex-wrap items-center gap-2">
		{#if playhead == null}
			<button
				class="flex items-center gap-1.5 px-3 h-9 rounded-lg text-sm text-neutral-200 bg-neutral-800 border border-neutral-600 hover:bg-neutral-700"
				on:click={preview}
				title="Plays in your headphones only"
			>
				<Play size="14" weight="fill" /> Play trimmed
			</button>
		{:else}
			<button
				class="flex items-center gap-1.5 px-3 h-9 rounded-lg text-sm text-neutral-200 bg-neutral-800 border border-neutral-600 hover:bg-neutral-700"
				on:click={stopPreview}
			>
				<Stop size="14" weight="fill" /> Stop
			</button>
		{/if}
		<button class="px-3 h-9 rounded-lg text-sm text-neutral-300 hover:bg-neutral-800 disabled:opacity-40" disabled={!trimmed} on:click={() => emit(0, total)}
			>Reset</button
		>
		<div class="grow"></div>
		<label class="flex items-center gap-2 text-sm text-neutral-300 cursor-pointer" title="A very short fade at the end, so a cut never clicks">
			<input
				type="checkbox"
				class="accent-[var(--orvo-accent)] w-4 h-4"
				checked={sound.fade_out ?? true}
				on:change={(e) => emit(start, end, e.currentTarget.checked)}
			/>
			Fade out
		</label>
	</div>
	<p class="text-xs text-neutral-500">Drag the handles (or use the arrow keys on them). Your original file is kept, so you can always undo this.</p>
</div>
