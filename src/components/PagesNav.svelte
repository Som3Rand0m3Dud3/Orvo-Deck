<script lang="ts">
	// Left sidebar "Pages": each profile of the device is a page. The pad's ◀ ▶ keys flip through them.
	import type { DeviceInfo } from "$lib/DeviceInfo";
	import type { Profile } from "$lib/Profile";

	import { profileManager } from "$lib/singletons";

	import { invoke } from "@tauri-apps/api/core";

	export let device: DeviceInfo;
	export let profile: Profile | undefined;

	let pages: string[] = [];
	async function refresh(_id?: string) {
		if (!device) return;
		try {
			pages = ((await invoke("get_profiles", { device: device.id })) as string[]).sort((a, b) => a.localeCompare(b));
		} catch {
			pages = [];
		}
	}
	$: refresh(profile?.id);

	const label = (id: string) => (id.includes("/") ? id.split("/")[1] : id);
</script>

<p class="orvo-label px-2 mb-2">Pages</p>
<div class="flex flex-col gap-1">
	{#each pages as id, i}
		<button
			class="flex items-center gap-3 min-h-11 px-3 rounded-xl border text-left transition-colors"
			class:border-blue-500={profile?.id == id}
			class:bg-neutral-700={profile?.id == id}
			class:text-neutral-100={profile?.id == id}
			class:border-transparent={profile?.id != id}
			class:text-neutral-300={profile?.id != id}
			class:hover:bg-neutral-800={profile?.id != id}
			on:click={() => $profileManager?.setProfile(id)}
		>
			<span
				class="flex items-center justify-center w-6 h-6 rounded-md text-xs font-bold"
				style={profile?.id == id ? "background: var(--orvo-accent); color: #fff" : ""}
				class:bg-neutral-700={profile?.id != id}>{i + 1}</span
			>
			<span class="truncate">{label(id)}</span>
		</button>
	{/each}
	<button
		class="flex items-center gap-3 min-h-11 px-3 rounded-xl border border-dashed border-neutral-600 text-neutral-400 hover:text-neutral-200 transition-colors"
		on:click={() => $profileManager?.openManager()}
	>
		<span class="w-6 text-center text-lg leading-none">+</span>New page / manage
	</button>
</div>
