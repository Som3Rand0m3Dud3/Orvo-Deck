<script lang="ts">
	// Right-click a cube in the actions panel: your own picture and name for that action.
	import ImageSquare from "phosphor-svelte/lib/ImageSquare";

	import { actionLooks, lookKey, saveLook, toKeyPicture, updatePlacedKeys, type Look } from "$lib/actionLooks";
	import type { ActionInstance } from "$lib/ActionInstance";
	import { voiceIcon, type SoundAction } from "$lib/soundboard";

	import { createEventDispatcher } from "svelte";

	export let action: SoundAction;
	/** What the cube shows now without your changes. */
	export let defaultIcon: string;
	export let defaultName: string;

	const dispatch = createEventDispatcher<{ close: void }>();
	const key = lookKey(action);
	const saved: Look = $actionLooks[key] ?? {};
	let image: string | undefined = saved.image;
	let name = saved.name ?? "";
	let alsoPlaced = true;
	let error = "";
	let busy = false;
	let fileInput: HTMLInputElement;

	async function pickFile(event: Event) {
		const file = (event.currentTarget as HTMLInputElement).files?.[0];
		if (!file) return;
		error = "";
		try {
			image = await toKeyPicture(file);
		} catch (e: any) {
			error = e?.message ?? String(e);
		}
	}

	// keys that already use this exact cube: same action, same sound / voice
	function matches(i: ActionInstance): boolean {
		if (i.action.plugin != action.plugin || i.action.uuid != action.uuid) return false;
		const s: any = i.settings ?? {};
		if (action.orvoSound) return s.sound == action.orvoSound.id;
		if (action.orvoPreset) return s.preset == action.orvoPreset.settings.preset;
		return true;
	}

	// what a placed key looked like before any edit
	function original(i: ActionInstance): Look {
		const s: any = i.settings ?? {};
		if (action.orvoPreset) return { image: action.icon || voiceIcon(s.preset ?? "", s.name ?? ""), name: "" };
		return { image: i.action.states?.[0]?.image || i.action.icon, name: action.orvoSound ? action.orvoSound.name : (i.action.states?.[0]?.text ?? "") };
	}

	async function save(reset = false) {
		busy = true;
		const look: Look | null = reset ? null : { image, name: name.trim() || undefined };
		await saveLook(key, look);
		if (alsoPlaced) {
			await updatePlacedKeys(matches, (i) => {
				const o = original(i);
				return { image: look?.image ?? o.image, name: look?.name ?? o.name };
			});
		}
		busy = false;
		dispatch("close");
	}
</script>

<!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
<div class="fixed inset-0 z-[60] bg-black/50" on:click={() => dispatch("close")}></div>
<form
	role="dialog"
	aria-label="Edit action"
	class="fixed left-1/2 top-1/2 -translate-x-1/2 -translate-y-1/2 z-[61] w-[26rem] max-w-[calc(100%-2rem)] p-5 flex flex-col gap-4 rounded-2xl bg-neutral-900 border border-neutral-600 shadow-2xl"
	on:submit|preventDefault={() => save()}
>
	<div>
		<h2 class="font-bold text-neutral-100">Edit action</h2>
		<p class="text-sm text-neutral-400">Every key you put this on gets this picture and name.</p>
	</div>
	<div class="flex items-center gap-4">
		<img src={image ?? defaultIcon} alt="" class="w-24 h-24 rounded-xl border border-neutral-600 object-cover bg-neutral-800" />
		<div class="flex flex-col gap-2">
			<input bind:this={fileInput} type="file" accept="image/png,image/jpeg,image/gif,image/webp,image/svg+xml" class="hidden" on:change={pickFile} />
			<button
				type="button"
				class="flex items-center gap-1.5 px-3 h-9 rounded-lg text-sm text-neutral-200 bg-neutral-800 border border-neutral-600 hover:bg-neutral-700"
				on:click={() => fileInput.click()}
			>
				<ImageSquare size="16" /> Change picture…
			</button>
			{#if image}<button type="button" class="px-3 h-8 rounded-lg text-sm text-neutral-400 hover:bg-neutral-800" on:click={() => (image = undefined)}
					>Use the original picture</button
				>{/if}
		</div>
	</div>
	{#if error}<p class="text-sm text-red-400">{error}</p>{/if}
	<label class="flex flex-col gap-1.5 text-sm text-neutral-400">
		Name on the key
		<input bind:value={name} maxlength="40" placeholder={defaultName} class="h-10 px-3 rounded-lg bg-neutral-800 border border-neutral-600 text-neutral-100" />
		<span class="text-xs text-neutral-500">Leave empty to keep "{defaultName}". Pictures read better than words on the pad.</span>
	</label>
	<label class="flex items-center gap-2 text-sm text-neutral-300 cursor-pointer">
		<input type="checkbox" bind:checked={alsoPlaced} class="accent-[var(--orvo-accent)] w-4 h-4" />
		Also update keys already using it
	</label>
	<div class="flex items-center gap-2">
		<button
			type="button"
			class="px-3 h-9 rounded-lg text-sm text-neutral-400 hover:bg-neutral-800 disabled:opacity-40"
			disabled={busy || !$actionLooks[key]}
			on:click={() => save(true)}>Reset</button
		>
		<span class="grow"></span>
		<button type="button" class="px-3 h-9 rounded-lg text-sm text-neutral-300 hover:bg-neutral-800" on:click={() => dispatch("close")}>Cancel</button>
		<button
			type="submit"
			disabled={busy}
			class="px-4 h-9 rounded-lg text-sm font-semibold text-white disabled:opacity-60"
			style="background: var(--orvo-accent)">{busy ? "Saving…" : "Save"}</button
		>
	</div>
</form>
