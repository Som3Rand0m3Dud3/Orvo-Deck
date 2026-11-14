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
	import OrvoDialog from "../components/OrvoDialog.svelte";
	import PadPreloader from "../components/PadPreloader.svelte";
	import PagesNav from "../components/PagesNav.svelte";
	import PanelResizer from "../components/PanelResizer.svelte";
	import ParentActionView from "../components/ParentActionView.svelte";
	import PluginManager from "../components/PluginManager.svelte";
	import ProfileManager from "../components/ProfileManager.svelte";
	import PropertyInspectorView from "../components/PropertyInspectorView.svelte";
	import SettingsView from "../components/SettingsView.svelte";
	import SoundboardView from "../components/SoundboardView.svelte";
	import { appView } from "$lib/soundboard";

	let devices: { [id: string]: DeviceInfo } = {};
	let selectedDevice: string;
	let selectedProfiles: { [id: string]: Profile } = {};

	initPortBase();

	// Side panel widths (drag the inner edge; double-click it to reset).
	const LEFT_DEFAULT = 288;
	const RIGHT_DEFAULT = 368;
	let leftWidth = LEFT_DEFAULT;
	let rightWidth = RIGHT_DEFAULT;

	// Remember the pad even while it briefly disconnects (e.g. during a firmware update).
	let lastDevice = "";
	$: if (selectedDevice) lastDevice = selectedDevice;
	$: hasDevice = Object.keys(devices).length > 0 && selectedDevice && devices[selectedDevice];
</script>

<svelte:window on:dragover={(event) => event.preventDefault()} on:drop={(event) => event.preventDefault()} />

<div class="relative flex flex-col h-screen text-neutral-300" style="background: var(--orvo-stage)">
	<header class="flex items-center gap-4 h-14 px-5 shrink-0 bg-neutral-900 border-b border-neutral-700">
		<div class="flex items-center gap-2.5">
			<img src="/orvo-logo.svg" width="28" height="28" alt="" aria-hidden="true" />
			<span class="font-bold text-base text-neutral-100 tracking-tight">Orvo Studio</span>
		</div>

		<div class="flex items-center gap-2 h-9 px-3 rounded-full bg-neutral-800 border border-neutral-700">
			<span class="w-2 h-2 rounded-full" class:bg-emerald-400={hasDevice} class:bg-neutral-500={!hasDevice}></span>
			<DeviceSelector bind:devices bind:value={selectedDevice} bind:selectedProfiles bind:this={$deviceSelector} />
			{#if !hasDevice}<span class="text-sm text-neutral-400">No device</span>{/if}
		</div>

		<div role="tablist" aria-label="Pages" class="flex gap-1 p-1 rounded-xl bg-neutral-800 border border-neutral-700">
			<button
				role="tab"
				aria-selected={$appView == "pad"}
				class="px-4 h-8 rounded-lg text-sm transition-colors {$appView == 'pad'
					? 'bg-neutral-950 text-neutral-100'
					: 'text-neutral-300 hover:text-neutral-100'}"
				on:click={() => appView.set("pad")}>{(hasDevice && devices[selectedDevice]?.name) || "Orvo M1"}</button
			>
			<button
				role="tab"
				aria-selected={$appView == "soundboard"}
				class="px-4 h-8 rounded-lg text-sm transition-colors {$appView == 'soundboard'
					? 'bg-neutral-950 text-neutral-100'
					: 'text-neutral-300 hover:text-neutral-100'}"
				on:click={() => appView.set("soundboard")}>Soundboard</button
			>
		</div>

		<div class="grow"></div>
		<PluginManager />
		<SettingsView />
	</header>

	{#if $appView == "soundboard"}
		<SoundboardView />
	{/if}
	<div class="flex flex-row grow min-h-0" class:hidden={$appView == "soundboard"}>
		<div class="relative flex shrink-0 min-h-0" style="width: {leftWidth}px">
			<aside class="flex flex-col w-full bg-neutral-900 border-r border-neutral-700 min-h-0">
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
				<div class="grow"></div>
				<AppUpdateCard />
				{#if lastDevice}<PadFirmware device={lastDevice} />{/if}
				{#each Object.keys(devices) as id (id)}
					<PadPreloader device={id} profile={selectedProfiles[id]} />
				{/each}
			</aside>
			<PanelResizer bind:width={leftWidth} min={220} max={480} initial={LEFT_DEFAULT} edge="right" storageKey="orvo-left-width" label="Resize left panel" />
		</div>

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
			<ActionList bind:this={$actionList} />
		</main>

		<div class="relative flex shrink-0 min-h-0" style="width: {rightWidth}px">
			<PanelResizer bind:width={rightWidth} min={300} max={640} initial={RIGHT_DEFAULT} edge="left" storageKey="orvo-right-width" label="Resize right panel" />
			<aside class="relative flex flex-col w-full bg-neutral-900 border-l border-neutral-700 min-h-0 overflow-auto">
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
</div>

<OrvoDialog />
