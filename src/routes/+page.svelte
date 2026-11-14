<script lang="ts">
	import type { DeviceInfo } from "$lib/DeviceInfo";
	import type { Profile } from "$lib/Profile";

	import { initPortBase } from "$lib/ports";
	import { inspectedInstance, inspectedParentAction } from "$lib/propertyInspector";
	import { actionList, deviceSelector, profileManager } from "$lib/singletons";

	import ActionList from "../components/ActionList.svelte";
	import BoxEditor from "../components/BoxEditor.svelte";
	import DeviceSelector from "../components/DeviceSelector.svelte";
	import DeviceView from "../components/DeviceView.svelte";
	import NoDevicesDetected from "../components/NoDevicesDetected.svelte";
	import PadFirmware from "../components/PadFirmware.svelte";
	import AppUpdateCard from "../components/AppUpdateCard.svelte";
	import PadPreloader from "../components/PadPreloader.svelte";
	import PagesNav from "../components/PagesNav.svelte";
	import ParentActionView from "../components/ParentActionView.svelte";
	import PluginManager from "../components/PluginManager.svelte";
	import ProfileManager from "../components/ProfileManager.svelte";
	import PropertyInspectorView from "../components/PropertyInspectorView.svelte";
	import SettingsView from "../components/SettingsView.svelte";

	let devices: { [id: string]: DeviceInfo } = {};
	let selectedDevice: string;
	let selectedProfiles: { [id: string]: Profile } = {};

	initPortBase();

	// Remember the pad even while it briefly disconnects (e.g. during a firmware update).
	let lastDevice = "";
	$: if (selectedDevice) lastDevice = selectedDevice;
	$: hasDevice = Object.keys(devices).length > 0 && selectedDevice && devices[selectedDevice];
</script>

<svelte:window on:dragover={(event) => event.preventDefault()} on:drop={(event) => event.preventDefault()} />

<div class="relative flex flex-col h-screen text-neutral-300" style="background: var(--orvo-stage)">
	<header class="flex items-center gap-4 h-14 px-5 shrink-0 bg-neutral-900 border-b border-neutral-700">
		<div class="flex items-center gap-2.5">
			<svg width="28" height="28" viewBox="0 0 28 28" aria-hidden="true">
				<rect width="28" height="28" rx="8" class="fill-neutral-800" />
				<circle cx="14" cy="14" r="7" fill="none" stroke="var(--orvo-accent)" stroke-width="3" />
				<circle cx="20" cy="8" r="3" fill="var(--orvo-orange)" />
			</svg>
			<span class="font-bold text-base text-neutral-100 tracking-tight">Orvo Studio</span>
		</div>

		<div class="flex items-center gap-2 h-9 px-3 rounded-full bg-neutral-800 border border-neutral-700">
			<span class="w-2 h-2 rounded-full" class:bg-emerald-400={hasDevice} class:bg-neutral-500={!hasDevice}></span>
			<DeviceSelector bind:devices bind:value={selectedDevice} bind:selectedProfiles bind:this={$deviceSelector} />
			{#if !hasDevice}<span class="text-sm text-neutral-400">No device</span>{/if}
		</div>

		<div class="grow"></div>
		<PluginManager />
		<SettingsView />
	</header>

	<div class="flex flex-row grow min-h-0">
		<aside class="flex flex-col w-72 shrink-0 bg-neutral-900 border-r border-neutral-700 min-h-0">
			<div class="p-3">
				{#key selectedDevice}
					{#if hasDevice}
						<PagesNav device={devices[selectedDevice]} profile={selectedProfiles[selectedDevice]} />
						<ProfileManager
							showSelect={false}
							bind:device={devices[selectedDevice]}
							bind:profile={selectedProfiles[selectedDevice]}
							bind:this={$profileManager}
						/>
					{/if}
				{/key}
			</div>
			<p class="orvo-label px-5 pt-2 pb-2 border-t border-neutral-700">Actions</p>
			<ActionList bind:this={$actionList} />
			<AppUpdateCard />
			{#if lastDevice}<PadFirmware device={lastDevice} />{/if}
			{#each Object.keys(devices) as id (id)}
				<PadPreloader device={id} profile={selectedProfiles[id]} />
			{/each}
		</aside>

		<main class="relative flex flex-col grow min-w-0">
			{#if Object.keys(devices).length > 0 && selectedProfiles}
				{#if $inspectedParentAction}
					<ParentActionView bind:profile={selectedProfiles[selectedDevice]} />
				{/if}

				{#each Object.entries(devices) as [id, device]}
					{#if device && selectedProfiles[id]}
						<DeviceView bind:device bind:profile={selectedProfiles[id]} bind:selectedDevice />
					{/if}
				{/each}
			{:else}
				<NoDevicesDetected />
			{/if}
		</main>

		<aside class="relative flex flex-col w-[23rem] shrink-0 bg-neutral-900 border-l border-neutral-700 min-h-0 overflow-auto">
			{#if hasDevice && selectedProfiles[selectedDevice]}
				<BoxEditor bind:profile={selectedProfiles[selectedDevice]} />
				<div class:hidden={typeof $inspectedInstance != "string"} class="flex flex-col grow border-t border-neutral-700">
					<p class="orvo-label px-5 pt-4 pb-1">Action settings</p>
					<PropertyInspectorView bind:device={devices[selectedDevice]} bind:profile={selectedProfiles[selectedDevice]} />
				</div>
			{:else}
				<BoxEditor profile={null} />
			{/if}
		</aside>
	</div>
</div>
