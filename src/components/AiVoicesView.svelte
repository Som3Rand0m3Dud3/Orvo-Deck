<script lang="ts">
	// AI voices (RVC): one-time engine download, then voice models as picture tiles.
	// One voice at a time across effects and AI; an AI voice also pauses the Mixer's effects.
	import DownloadSimple from "phosphor-svelte/lib/DownloadSimple";
	import Gear from "phosphor-svelte/lib/Gear";
	import Headphones from "phosphor-svelte/lib/Headphones";
	import Plus from "phosphor-svelte/lib/Plus";
	import Trash from "phosphor-svelte/lib/Trash";
	import X from "phosphor-svelte/lib/X";

	import { toKeyPicture } from "$lib/actionLooks";
	import { refreshSoundboard, soundboard, voiceIcon, type AiState, type AiVoice } from "$lib/soundboard";

	import { invoke } from "@tauri-apps/api/core";
	import { listen } from "@tauri-apps/api/event";
	import { open } from "@tauri-apps/plugin-dialog";
	import { onDestroy, onMount } from "svelte";

	let ai: AiState | null = null;
	let progress: { done: number; total: number } | null = null;
	let error = "";
	let busy = false;

	async function refresh() {
		try {
			ai = await invoke<AiState>("sb_ai_state");
		} catch {
			// not available (older backend)
		}
	}
	let timer: ReturnType<typeof setInterval>;
	let unlisten: (() => void) | undefined;
	onMount(async () => {
		refresh();
		timer = setInterval(refresh, 1000);
		unlisten = await listen<{ done: number; total: number }>("ai_install_progress", (e) => (progress = e.payload));
	});
	onDestroy(() => {
		flushPatch();
		clearInterval(timer);
		unlisten?.();
		if (listening) invoke("sb_listen", { on: false });
	});

	$: config = $soundboard?.config;
	$: voices = config?.ai_voices ?? [];
	$: current = config?.voice_preset ?? "off";
	$: status = ai?.status;
	$: activeAi = current.startsWith("ai:");
	const icon = (v: AiVoice) => voiceIcon(`ai:${v.id}`, v.name, v.image);
	const mb = (b: number) => Math.round(b / 1e6);

	async function install() {
		error = "";
		progress = { done: 0, total: (ai?.download_mb ?? 580) * 1e6 };
		try {
			await invoke("sb_ai_install");
		} catch (e) {
			error = String(e);
		}
		progress = null;
		refresh();
	}

	async function addVoice() {
		error = "";
		const picked = await open({ multiple: false, directory: false, filters: [{ name: "AI voice", extensions: ["pth", "onnx", "zip"] }] });
		if (!picked || Array.isArray(picked)) return;
		busy = true;
		try {
			const v = await invoke<AiVoice>("sb_ai_add", { path: picked });
			await refreshSoundboard();
			editing = v;
		} catch (e) {
			error = String(e);
		}
		busy = false;
	}

	// drop zone: any number of .pth / .onnx / .zip files, added one after another
	let dragging = false;
	let dropNote = "";
	function onDragLeave(e: DragEvent) {
		if (!(e.currentTarget as HTMLElement).contains(e.relatedTarget as Node)) dragging = false;
	}
	async function onDrop(e: DragEvent) {
		dragging = false;
		const files = [...(e.dataTransfer?.files ?? [])];
		if (!files.length || busy) return;
		error = "";
		busy = true;
		const failed: string[] = [];
		let added = 0;
		for (const [i, f] of files.entries()) {
			dropNote = files.length > 1 ? `Adding ${i + 1} of ${files.length}: ${f.name}` : `Adding ${f.name}`;
			try {
				const data = new Uint8Array(await f.arrayBuffer());
				await invoke<AiVoice>("sb_ai_add_bytes", data, { headers: { "x-file-name": encodeURIComponent(f.name) } });
				added++;
			} catch (err) {
				failed.push(`${f.name}: ${err}`);
			}
		}
		await refreshSoundboard();
		if (failed.length) error = (added ? `Added ${added}. ` : "") + `Couldn't add ${failed.join(" · ")}`;
		dropNote = "";
		busy = false;
	}

	async function pick(v: AiVoice) {
		const id = `ai:${v.id}`;
		await invoke("sb_set_voice", { id: current == id ? "off" : id });
		refreshSoundboard();
		setTimeout(refresh, 300);
	}

	let listening = false;
	function toggleListen() {
		listening = !listening;
		invoke("sb_listen", { on: listening });
	}

	// settings dialog
	let editing: AiVoice | null = null;
	$: if (editing) editing = voices.find((v) => v.id == editing!.id) ?? null;
	type Patch = Partial<{ name: string; pitch: number; quality: string; image: string }>;
	let pending: { id: string; patch: Patch } | null = null;
	let patchTimer: ReturnType<typeof setTimeout> | null = null;
	function flushPatch() {
		if (patchTimer) clearTimeout(patchTimer);
		patchTimer = null;
		if (pending) invoke("sb_ai_update", pending).catch((e) => (error = String(e)));
		pending = null;
	}
	function patch(p: Patch) {
		if (!editing) return;
		const id = editing.id;
		soundboard.update((s) => (s ? { ...s, config: { ...s.config, ai_voices: s.config.ai_voices.map((v) => (v.id == id ? { ...v, ...p } : v)) } } : s));
		if (pending && pending.id != id) flushPatch();
		pending = { id, patch: { ...(pending?.patch ?? {}), ...p } };
		if (patchTimer) clearTimeout(patchTimer);
		patchTimer = setTimeout(flushPatch, 150);
	}
	let fileInput: HTMLInputElement;
	async function onPicture(e: Event) {
		const f = (e.currentTarget as HTMLInputElement).files?.[0];
		if (f) patch({ image: await toKeyPicture(f) });
	}
	async function remove(v: AiVoice) {
		editing = null;
		await invoke("sb_ai_remove", { id: v.id });
		refreshSoundboard();
	}
	const num = (e: Event) => Number((e.currentTarget as HTMLInputElement).value);
	const pitchText = (p: number) => (p == 0 ? "No change" : `${p > 0 ? "+" : "−"}${Math.abs(p)} semitones`);
	const QUALITY = [
		["fast", "Fast", "Least delay, rougher"],
		["balanced", "Balanced", "Good for talking"],
		["best", "Best", "Cleanest, more delay"],
	];
</script>

<svelte:window on:keydown={(e) => e.key == "Escape" && (editing = null)} />

<div class="flex flex-col grow min-h-0 gap-5 p-5 overflow-auto">
	<div class="flex flex-wrap items-end gap-3">
		<div class="grow">
			<h1 class="text-xl font-bold text-neutral-100">AI voices</h1>
			<p class="text-sm text-neutral-400">Sound like a different person. Runs on your PC's graphics card; nothing is sent online.</p>
		</div>
		{#if ai?.installed}
			<button
				class="flex items-center gap-1.5 px-3.5 h-10 rounded-lg text-sm border {listening
					? 'text-white border-transparent'
					: 'text-neutral-200 bg-neutral-800 border-neutral-600 hover:bg-neutral-700'}"
				style={listening ? "background: var(--orvo-accent)" : ""}
				aria-pressed={listening}
				on:click={toggleListen}><Headphones size="16" /> {listening ? "Listening…" : "Hear it"}</button
			>
		{/if}
	</div>

	{#if error}
		<p role="alert" class="px-3.5 py-2.5 rounded-xl border border-red-800/60 bg-red-950/30 text-sm text-red-200">{error}</p>
	{/if}

	{#if !ai}
		<p class="text-neutral-400">Loading…</p>
	{:else if !ai.supported}
		<p class="text-neutral-300">AI voices need Windows.</p>
	{:else if !ai.installed}
		<div class="flex flex-col items-start gap-4 max-w-xl p-6 rounded-2xl border border-neutral-700 bg-neutral-900">
			<h2 class="text-lg font-bold text-neutral-100">Get the AI engine</h2>
			<p class="text-sm text-neutral-300 leading-relaxed">
				A one-time download of about <strong>{ai.download_mb} MB</strong>. It uses about 1–2 GB of graphics memory, and only while an AI voice is on.
			</p>
			{#if progress || ai.installing}
				{@const pct = progress && progress.total ? Math.min(100, (progress.done / progress.total) * 100) : 0}
				<div class="w-full flex flex-col gap-1.5">
					<div
						class="h-2.5 w-full rounded-full bg-neutral-800 overflow-hidden"
						role="progressbar"
						aria-valuenow={Math.round(pct)}
						aria-valuemin={0}
						aria-valuemax={100}
					>
						<div class="h-full rounded-full transition-all" style="width: {pct}%; background: var(--orvo-accent)"></div>
					</div>
					<span class="text-xs text-neutral-400">{progress ? `${mb(progress.done)} of ${mb(progress.total)} MB` : "Downloading…"}</span>
				</div>
			{:else}
				<button class="flex items-center gap-2 px-4 h-10 rounded-lg text-sm font-semibold text-white" style="background: var(--orvo-accent)" on:click={install}
					><DownloadSimple size="16" /> Download ({ai.download_mb} MB)</button
				>
			{/if}
		</div>
	{:else}
		{#if activeAi && status}
			<div role="status" class="flex flex-wrap items-center gap-x-5 gap-y-1 px-3.5 py-2.5 rounded-xl border border-neutral-700 bg-neutral-900 text-sm">
				{#if status.state == "loading"}
					<span class="text-neutral-200">Loading the voice…</span>
				{:else if status.state == "error"}
					<span class="text-red-300">Problem: {status.message}</span>
				{:else if status.state == "running"}
					<span class="text-neutral-100">{status.gpu ? "Running on your graphics card ✓" : "Running on the CPU (no graphics card found)"}</span>
					{#if status.compute_ms > 0}
						<span class="text-neutral-400">Delay about <strong class="text-neutral-200">{Math.round(status.delay_ms)} ms</strong></span>
						<span class="text-neutral-400"
							>Load <strong class={status.compute_ms > status.block_ms * 0.8 ? "text-amber-300" : "text-neutral-200"}
								>{Math.round((status.compute_ms / status.block_ms) * 100)}%</strong
							></span
						>
					{/if}
				{/if}
			</div>
			{#if status.state == "running" && status.compute_ms > status.block_ms * 0.8}
				<p class="text-xs text-amber-300">Your PC is struggling to keep up. Pick "Best" quality (bigger blocks) in the voice's settings.</p>
			{/if}
		{/if}

		<div role="radiogroup" aria-label="AI voices" class="grid gap-3" style="grid-template-columns: repeat(auto-fill, minmax(9.5rem, 1fr))">
			{#each voices as v (v.id)}
				{@const on = current == `ai:${v.id}`}
				<div class="relative group">
					<button
						role="radio"
						aria-checked={on}
						class="w-full flex flex-col items-center gap-2 pt-5 pb-3.5 px-2 rounded-2xl border-2 transition-colors {on
							? 'bg-[#1D2A44]'
							: 'bg-neutral-900 border-neutral-700 hover:border-neutral-500'}"
						style={on ? "border-color: var(--orvo-accent)" : ""}
						on:click={() => pick(v)}
					>
						<img src={icon(v)} alt="" class="w-20 h-20 rounded-2xl object-cover pointer-events-none" />
						<span class="text-base font-semibold text-neutral-100 text-center">{v.name}</span>
						<span class="text-xs text-neutral-400">{v.pitch ? pitchText(v.pitch) : "AI voice"}</span>
						{#if on}
							<span
								class="absolute top-2 left-2 px-2 py-0.5 rounded-full text-[10px] font-bold tracking-wider text-white"
								style="background: var(--orvo-accent)">ON</span
							>
						{/if}
					</button>
					<button
						class="absolute top-2 right-2 p-1.5 rounded-lg bg-neutral-800/90 text-neutral-300 hover:text-neutral-100 {on
							? ''
							: 'opacity-0 group-hover:opacity-100 focus:opacity-100'}"
						aria-label="Settings for {v.name}"
						title="Settings"
						on:click={() => (editing = v)}><Gear size="14" /></button
					>
				</div>
			{/each}
			<button
				class="flex flex-col items-center justify-center gap-2 min-h-[11.5rem] rounded-2xl border-2 border-dashed border-neutral-600 text-neutral-300 hover:border-neutral-400 hover:text-neutral-100 disabled:opacity-50"
				disabled={busy}
				on:click={addVoice}
			>
				<Plus size="28" />
				<span class="text-sm font-semibold">{busy ? "Adding the voice…" : "Add voice"}</span>
				<span class="text-xs text-neutral-400">.pth, .onnx or .zip</span>
			</button>
		</div>

		<!-- svelte-ignore a11y-no-static-element-interactions -->
		<div
			class="flex flex-col items-center justify-center gap-1 py-7 rounded-2xl border-2 border-dashed transition-colors text-center {dragging
				? 'bg-[#1D2A44]'
				: 'bg-neutral-900/80 border-neutral-700'}"
			style={dragging ? "border-color: var(--orvo-accent)" : ""}
			on:dragenter|preventDefault={() => (dragging = true)}
			on:dragover|preventDefault={() => (dragging = true)}
			on:dragleave|preventDefault={onDragLeave}
			on:drop|preventDefault={onDrop}
		>
			<DownloadSimple size="26" class="text-neutral-400" />
			<span class="text-base font-semibold text-neutral-200">{busy ? dropNote || "Adding…" : dragging ? "Let go to add" : "Drop voices here to add them"}</span>
			<span class="text-xs text-neutral-400">.pth, .onnx or .zip files. Several at once is fine; .pth voices are converted for you.</span>
		</div>

		<details class="group max-w-3xl rounded-2xl border border-neutral-700 bg-neutral-900" open={voices.length == 0}>
			<summary class="flex items-center gap-2 px-4 py-3 cursor-pointer select-none text-sm font-semibold text-neutral-100">
				Where do I get voices?
				<span class="text-xs font-normal text-neutral-400 group-open:hidden">Show the guide</span>
			</summary>
			<div class="flex flex-col gap-3 px-4 pb-4 text-sm text-neutral-300 leading-relaxed">
				<ol class="flex flex-col gap-2 list-decimal pl-5">
					<li>
						Find a voice on
						<button class="underline text-neutral-100" on:click={() => invoke("open_url", { url: "https://voice-models.com" })}>Voice-Models.com</button>
						or
						<button class="underline text-neutral-100" on:click={() => invoke("open_url", { url: "https://huggingface.co/models?search=rvc" })}>Hugging Face</button>.
						Search for a name plus <strong>"RVC v2"</strong>.
					</li>
					<li>Download it. You'll get a <strong>.pth</strong> file or a <strong>.zip</strong>. No need to unzip it or convert anything.</li>
					<li>Drop it in the box above (or click <strong>Add voice</strong>). Then click the voice to turn it on and press <strong>Hear it</strong> to test.</li>
				</ol>
				<div class="flex flex-col gap-1 text-xs text-neutral-400">
					<p>Works with: RVC <strong class="text-neutral-300">v2</strong> voices at 32, 40 or 48 kHz. Most voices you'll find are these.</p>
					<p>Won't work: old <strong class="text-neutral-300">v1</strong> voices, "no f0" voices, and training files named like <strong class="text-neutral-300">G_2333.pth</strong>.</p>
					<p>The <strong class="text-neutral-300">.index</strong> file that comes with some voices isn't needed.</p>
					<p>Please don't use a real person's voice to fool people.</p>
				</div>
			</div>
		</details>
	{/if}
</div>

{#if editing}
	<!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
	<div class="fixed inset-0 z-40 bg-black/50" on:click={() => (editing = null)}></div>
	<div
		role="dialog"
		aria-label="Settings for {editing.name}"
		class="fixed left-1/2 top-1/2 -translate-x-1/2 -translate-y-1/2 z-50 w-[28rem] p-5 flex flex-col gap-4 rounded-2xl bg-neutral-900 border border-neutral-600 shadow-2xl"
	>
		<div class="flex items-center gap-3">
			<button class="relative shrink-0 rounded-xl" title="Change picture" aria-label="Change picture" on:click={() => fileInput.click()}>
				<img src={icon(editing)} alt="" class="w-14 h-14 rounded-xl object-cover" />
			</button>
			<input bind:this={fileInput} type="file" accept="image/*" class="hidden" on:change={onPicture} />
			<input
				value={editing.name}
				maxlength="40"
				aria-label="Voice name"
				class="grow h-10 px-3 rounded-lg bg-neutral-800 border border-neutral-600 text-base font-semibold text-neutral-100"
				on:change={(e) => patch({ name: e.currentTarget.value })}
			/>
			<button class="p-1.5 rounded-lg text-neutral-400 hover:text-neutral-100" aria-label="Close" on:click={() => (editing = null)}><X size="18" /></button>
		</div>
		<div class="flex gap-2 text-xs">
			<button class="underline text-neutral-300" on:click={() => fileInput.click()}>Change picture</button>
			{#if editing.image}<button class="underline text-neutral-400" on:click={() => patch({ image: "" })}>Remove picture</button>{/if}
		</div>
		<label class="flex flex-col gap-1.5">
			<span class="flex justify-between text-sm text-neutral-300">Pitch <span class="text-neutral-400">{pitchText(editing.pitch)}</span></span>
			<input
				type="range"
				min="-24"
				max="24"
				step="1"
				value={editing.pitch}
				on:input={(e) => patch({ pitch: num(e) })}
				on:dblclick={() => patch({ pitch: 0 })}
				style="accent-color: var(--orvo-accent)"
			/>
			<span class="text-xs text-neutral-500">Man using a woman's voice: about +12. Woman using a man's voice: about −12.</span>
		</label>
		<div class="flex flex-col gap-1.5">
			<span class="text-sm text-neutral-300">Quality</span>
			<div role="radiogroup" aria-label="Quality" class="grid grid-cols-3 gap-2">
				{#each QUALITY as [key, label, hint]}
					<button
						role="radio"
						aria-checked={editing.quality == key}
						class="flex flex-col items-center gap-0.5 py-2 rounded-lg border text-sm {editing.quality == key
							? 'text-white border-transparent'
							: 'text-neutral-200 bg-neutral-800 border-neutral-600 hover:bg-neutral-700'}"
						style={editing.quality == key ? "background: var(--orvo-accent)" : ""}
						on:click={() => patch({ quality: key })}><span class="font-semibold">{label}</span><span class="text-[11px] opacity-80">{hint}</span></button
					>
				{/each}
			</div>
		</div>
		<div class="flex items-center gap-2 pt-1">
			<button class="flex items-center gap-1.5 px-3 h-9 rounded-lg text-sm text-red-300 hover:bg-red-950/40" on:click={() => editing && remove(editing)}
				><Trash size="15" /> Delete voice</button
			>
			<div class="grow"></div>
			<button class="px-4 h-9 rounded-lg text-sm font-semibold text-white" style="background: var(--orvo-accent)" on:click={() => (editing = null)}>Done</button
			>
		</div>
	</div>
{/if}
