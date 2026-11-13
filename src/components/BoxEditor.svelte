<script lang="ts">
	// The right-hand "box editor": choose what a key's box shows on the screen
	// (the plugin's own live content, a symbol + word, a word only, or a picture).
	import type { ActionInstance } from "$lib/ActionInstance";
	import type { ActionState } from "$lib/ActionState";
	import type { Profile } from "$lib/Profile";

	import { localisations } from "$lib/settings";
	import { getWebserverUrl } from "$lib/ports";
	import { inspectedInstance } from "$lib/propertyInspector";
	import { resizeImage } from "$lib/rendererHelper";
	import { BOX_COLOURS, type BoxMode, SYMBOL_COLOURS, SYMBOLS, blankUrl, modeOf, symbolUrl } from "$lib/symbols";

	import { invoke } from "@tauri-apps/api/core";

	export let profile: Profile | null;

	let instance: ActionInstance | null = null;
	let position = -1;
	let controller = "";
	$: {
		instance = null;
		if (typeof $inspectedInstance == "string" && profile) {
			const split = $inspectedInstance.split(".");
			controller = split[2];
			position = parseInt(split[3]);
			const index = parseInt(split[4]);
			const array = controller == "Encoder" ? profile.sliders : profile.keys;
			const top = array?.[position];
			if (top) instance = index == 0 ? top : (top.children?.[index - 1] ?? null);
		}
	}

	let stateIndex = 0;
	let lastContext = "";
	$: if (instance && instance.context != lastContext) {
		lastContext = instance.context;
		stateIndex = instance.current_state ?? 0;
	}
	$: st = instance ? instance.states[stateIndex] : null;
	$: info = st ? modeOf(st.image_override) : { mode: "live" as BoxMode };
	$: actionName = instance ? ($localisations?.[instance.action.plugin]?.[instance.action.uuid]?.Name ?? instance.action.name) : "";
	$: icon = instance ? (instance.action.icon.startsWith("opendeck/") ? instance.action.icon.replace("opendeck", "") : getWebserverUrl(instance.action.icon)) : "";

	let symbolColour = "#FFFFFF";
	$: if (info.colour) symbolColour = info.colour;

	async function commit(changes: Partial<ActionState>) {
		if (!instance || !profile) return;
		Object.assign(instance.states[stateIndex], changes);
		await invoke("set_state", { context: instance.context, index: stateIndex, state: instance.states[stateIndex] });
		// Give the key a new object so its preview redraws right away.
		const array = controller == "Encoder" ? profile.sliders : profile.keys;
		if (array[position] && array[position]!.context == instance.context) {
			array[position] = { ...instance, states: [...instance.states] };
			instance = array[position];
		}
		profile = profile;
	}

	function setMode(mode: BoxMode) {
		if (!st) return;
		if (mode == "live") commit({ image_override: null, image_scale: 100, alignment: "bottom" });
		else if (mode == "symbol") commit({ image_override: symbolUrl(info.symbol ?? Object.keys(SYMBOLS)[0], symbolColour), image_scale: 58, alignment: "bottom", size: 10, show: true });
		else if (mode == "word") commit({ image_override: blankUrl(), image_scale: 100, alignment: "middle", size: 15, show: true });
		else fileInput.click();
	}

	let fileInput: HTMLInputElement;
	function pickPicture() {
		const file = fileInput.files?.[0];
		if (!file) return;
		const reader = new FileReader();
		reader.onload = async () => {
			const result = reader.result?.toString();
			if (!result) return;
			commit({ image_override: (await resizeImage(result)) ?? result, image_scale: 100 });
		};
		reader.readAsDataURL(file);
		fileInput.value = "";
	}

	const MODES: { id: BoxMode; label: string }[] = [
		{ id: "live", label: "Live" },
		{ id: "symbol", label: "Symbol" },
		{ id: "word", label: "Word" },
		{ id: "picture", label: "Picture" },
	];
</script>

<input bind:this={fileInput} type="file" accept="image/*" class="hidden" on:change={pickPicture} />

{#if !instance || !st}
	<div class="flex flex-col items-center justify-center h-full px-8 text-center text-neutral-400">
		<div class="mb-4 w-14 h-14 rounded-2xl border-2 border-dashed border-neutral-600"></div>
		<p class="font-semibold text-neutral-300">Click a box to edit it</p>
		<p class="mt-1 text-sm">Drag an action from the left onto any of the 9 boxes, then choose how it looks on your pad.</p>
	</div>
{:else}
	<div class="flex flex-col gap-5 p-5">
		<div class="flex items-center gap-3">
			<img src={icon} alt="" class="w-10 h-10 rounded-xl bg-neutral-800 border border-neutral-700" />
			<div class="min-w-0">
				<p class="font-semibold text-neutral-100 truncate">Key {position + 1} · {actionName}</p>
				<p class="text-sm text-neutral-400 truncate">{instance.action.plugin}</p>
			</div>
		</div>

		{#if instance.states.length > 1}
			<div>
				<p class="orvo-label mb-2">Look when</p>
				<div class="flex gap-2">
					{#each instance.states as _, i}
						<button
							class="flex-1 h-10 rounded-lg border text-sm transition-colors"
							class:border-blue-500={stateIndex == i}
							class:bg-neutral-700={stateIndex == i}
							class:text-neutral-100={stateIndex == i}
							class:border-neutral-700={stateIndex != i}
							class:text-neutral-400={stateIndex != i}
							on:click={() => (stateIndex = i)}
						>
							{i == 0 ? "Off" : i == 1 ? "On" : `State ${i + 1}`}
						</button>
					{/each}
				</div>
			</div>
		{/if}

		<div>
			<p class="orvo-label mb-2">Show on the screen</p>
			<div role="group" aria-label="Box content" class="grid grid-cols-4 gap-1 p-1 rounded-xl bg-neutral-800">
				{#each MODES as m}
					<button
						class="h-10 rounded-lg text-sm transition-colors"
						class:bg-neutral-600={info.mode == m.id}
						class:text-neutral-100={info.mode == m.id}
						class:font-semibold={info.mode == m.id}
						class:text-neutral-400={info.mode != m.id}
						on:click={() => setMode(m.id)}
					>
						{m.label}
					</button>
				{/each}
			</div>
			<p class="mt-2 text-xs text-neutral-400">
				{#if info.mode == "live"}What the plugin shows, like live numbers (CPU %, timers) or its own icon.
				{:else if info.mode == "symbol"}A symbol with a short word under it, for names too long to fit.
				{:else if info.mode == "word"}Just a word, nice and big.
				{:else}Your own picture. <button class="underline text-blue-400" on:click={() => fileInput.click()}>Choose another</button>{/if}
			</p>
		</div>

		{#if info.mode == "symbol"}
			<div>
				<p class="orvo-label mb-2">Symbol</p>
				<div class="grid grid-cols-6 gap-1.5 max-h-48 overflow-auto pr-1">
					{#each Object.entries(SYMBOLS) as [name, d]}
						<button
							class="aspect-square flex items-center justify-center rounded-lg border transition-colors"
							class:border-blue-500={info.symbol == name}
							class:bg-neutral-700={info.symbol == name}
							class:border-neutral-700={info.symbol != name}
							title={name}
							aria-label={name}
							on:click={() => commit({ image_override: symbolUrl(name, symbolColour) })}
						>
							<svg viewBox="0 0 24 24" class="w-5 h-5" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path {d} /></svg>
						</button>
					{/each}
				</div>
				<div class="flex gap-1.5 mt-2">
					{#each SYMBOL_COLOURS as c}
						<button
							class="w-7 h-7 rounded-full border-2"
							style="background: {c}"
							class:border-blue-500={symbolColour.toUpperCase() == c}
							class:border-transparent={symbolColour.toUpperCase() != c}
							aria-label="Symbol colour {c}"
							on:click={() => {
								symbolColour = c;
								commit({ image_override: symbolUrl(info.symbol ?? "star", c) });
							}}
						></button>
					{/each}
				</div>
			</div>
		{/if}

		{#if info.mode != "picture"}
			<label class="flex flex-col gap-2">
				<span class="orvo-label">{info.mode == "symbol" ? "Word under the symbol" : "Word"}</span>
				<input
					value={st.text_override ?? ""}
					placeholder={st.text || actionName}
					maxlength="24"
					class="h-11 px-3 rounded-lg bg-neutral-800 border border-neutral-700 text-neutral-100"
					on:change={(e) => commit({ text_override: e.currentTarget.value || null, show: true })}
				/>
			</label>
		{/if}

		<div>
			<p class="orvo-label mb-2">Box colour</p>
			<div class="flex gap-1.5 items-center">
				{#each BOX_COLOURS as c}
					<button
						class="w-8 h-8 rounded-lg border-2"
						style="background: {c}"
						class:border-blue-500={st.background_colour.toUpperCase().startsWith(c)}
						class:border-neutral-600={!st.background_colour.toUpperCase().startsWith(c)}
						aria-label="Box colour {c}"
						on:click={() => commit({ background_colour: c })}
					></button>
				{/each}
				<input
					type="color"
					value={st.background_colour.slice(0, 7)}
					class="w-8 h-8 rounded-lg bg-transparent"
					aria-label="Custom box colour"
					on:change={(e) => commit({ background_colour: e.currentTarget.value })}
				/>
			</div>
		</div>

		<p class="text-xs text-neutral-400">More options (font, outline, size) are under right-click → Edit on the box.</p>
	</div>
{/if}
