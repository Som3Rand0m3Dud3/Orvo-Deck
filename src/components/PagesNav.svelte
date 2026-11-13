<script lang="ts">
	// Left sidebar "Pages": each profile of the device is a page. The pad's ◀ ▶ keys flip through them.
	// Right-click a page to rename, duplicate or delete it.
	import type { DeviceInfo } from "$lib/DeviceInfo";
	import type { Profile } from "$lib/Profile";

	import { profileManager } from "$lib/singletons";

	import { invoke } from "@tauri-apps/api/core";
	import { ask, message } from "@tauri-apps/plugin-dialog";
	import { tick } from "svelte";

	export let device: DeviceInfo;
	export let profile: Profile | undefined;

	let pages: string[] = [];
	async function refresh(_id?: string) {
		if (!device) return;
		try {
			pages = [...new Set((await invoke("get_profiles", { device: device.id })) as string[])].sort((a, b) => a.localeCompare(b));
		} catch {
			pages = [];
		}
	}
	$: refresh(profile?.id);

	const label = (id: string) => (id.includes("/") ? id.split("/")[1] : id);

	let menu: { id: string; x: number; y: number } | null = null;
	let renaming: string | null = null;
	let newName = "";
	let renameInput: HTMLInputElement;

	function openMenu(event: MouseEvent, id: string) {
		event.preventDefault();
		menu = { id, x: event.clientX, y: event.clientY };
	}

	async function startRename(id: string) {
		menu = null;
		renaming = id;
		newName = label(id);
		await tick();
		renameInput?.select();
	}

	async function finishRename() {
		const id = renaming;
		renaming = null;
		if (!id) return;
		const name = newName.trim();
		if (!name || name == label(id)) return;
		if (!/^[a-zA-Z0-9_ ]+$/.test(name)) {
			message("Page names can use letters, numbers, spaces and _ only.", { title: "Rename page", kind: "warning" });
			return;
		}
		const newId = id.includes("/") ? `${id.split("/")[0]}/${name}` : name;
		if (pages.includes(newId)) {
			message(`There is already a page called "${name}".`, { title: "Rename page", kind: "warning" });
			return;
		}
		try {
			await invoke("rename_profile", { device: device.id, oldId: id, newId, retain: false });
			if (profile?.id == id) await $profileManager?.setProfile(newId);
		} catch (error) {
			message(String(error), { title: "Rename page", kind: "error" });
		}
		$profileManager?.reload();
		refresh();
	}

	// "+ New page": make the page straight away and let you name it in place.
	async function newPage() {
		let n = pages.length + 1;
		let id = `Page ${n}`;
		while (pages.includes(id)) id = `Page ${++n}`;
		try {
			await $profileManager?.setProfile(id);
			await new Promise((r) => setTimeout(r, 120));
			await refresh();
			$profileManager?.reload();
			startRename(id);
		} catch (error) {
			message(String(error), { title: "New page", kind: "error" });
		}
	}

	async function duplicate(id: string) {
		menu = null;
		let n = 1;
		let newId = `${id} copy`;
		while (pages.includes(newId)) newId = `${id} copy ${++n}`;
		try {
			await invoke("rename_profile", { device: device.id, oldId: id, newId, retain: true });
		} catch (error) {
			message(String(error), { title: "Duplicate page", kind: "error" });
		}
		$profileManager?.reload();
		refresh();
	}

	async function remove(id: string) {
		menu = null;
		if (pages.length < 2) {
			message("This is your only page, so it can't be deleted.", { title: "Delete page", kind: "info" });
			return;
		}
		if (!(await ask(`Delete the page "${label(id)}" and everything on it?\n\nThis can't be undone.`, { title: "Delete page", kind: "warning" }))) return;
		try {
			if (profile?.id == id) {
				const others = pages.filter((p) => p != id);
				const next = others[Math.min(pages.indexOf(id), others.length - 1)];
				await $profileManager?.setProfile(next);
				await new Promise((r) => setTimeout(r, 150));
			}
			await invoke("delete_profile", { device: device.id, profile: id });
		} catch (error) {
			message(String(error), { title: "Delete page", kind: "error" });
		}
		$profileManager?.reload();
		refresh();
	}
</script>

<svelte:window
	on:click={() => (menu = null)}
	on:keydown={(e) => {
		if (e.key == "Escape") {
			menu = null;
			renaming = null;
		}
	}}
/>

<p class="orvo-label px-2 mb-2">Pages</p>
<div class="flex flex-col gap-1">
	{#each pages as id, i}
		{#if renaming == id}
			<input
				bind:this={renameInput}
				bind:value={newName}
				class="min-h-11 px-3 rounded-xl border border-blue-500 bg-neutral-800 text-neutral-100"
				aria-label="New page name"
				on:keydown={(e) => {
					if (e.key == "Enter") finishRename();
				}}
				on:blur={finishRename}
			/>
		{:else}
			<button
				class="flex items-center gap-3 min-h-11 px-3 rounded-xl border text-left transition-colors"
				class:border-blue-500={profile?.id == id}
				class:bg-neutral-700={profile?.id == id}
				class:text-neutral-100={profile?.id == id}
				class:border-transparent={profile?.id != id}
				class:text-neutral-300={profile?.id != id}
				class:hover:bg-neutral-800={profile?.id != id}
				title="Right-click to rename, duplicate or delete"
				on:click={() => $profileManager?.setProfile(id)}
				on:contextmenu={(e) => openMenu(e, id)}
			>
				<span
					class="flex items-center justify-center w-6 h-6 rounded-md text-xs font-bold"
					style={profile?.id == id ? "background: var(--orvo-accent); color: #fff" : ""}
					class:bg-neutral-700={profile?.id != id}>{i + 1}</span
				>
				<span class="truncate">{label(id)}</span>
			</button>
		{/if}
	{/each}
	<button
		class="flex items-center gap-3 min-h-11 px-3 rounded-xl border border-dashed border-neutral-600 text-neutral-400 hover:text-neutral-200 transition-colors"
		on:click={newPage}
	>
		<span class="w-6 text-center text-lg leading-none">+</span>New page
	</button>
</div>

{#if menu}
	<div
		role="menu"
		class="fixed z-50 w-40 py-1 rounded-xl bg-neutral-800 border border-neutral-600 text-sm text-neutral-200 shadow-xl"
		style="left: {menu.x}px; top: {menu.y}px"
	>
		<button role="menuitem" class="w-full px-3 py-2 text-left hover:bg-neutral-700" on:click|stopPropagation={() => { if (menu) $profileManager?.setProfile(menu.id); menu = null; }}>Edit</button>
		<button role="menuitem" class="w-full px-3 py-2 text-left hover:bg-neutral-700" on:click|stopPropagation={() => menu && startRename(menu.id)}>Rename</button>
		<button role="menuitem" class="w-full px-3 py-2 text-left hover:bg-neutral-700" on:click|stopPropagation={() => menu && duplicate(menu.id)}>Duplicate</button>
		<button role="menuitem" class="w-full px-3 py-2 text-left text-red-400 hover:bg-neutral-700" on:click|stopPropagation={() => menu && remove(menu.id)}>Delete</button>
	</div>
{/if}
