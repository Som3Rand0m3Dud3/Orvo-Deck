<script lang="ts">
	import type { ActionInstance } from "$lib/ActionInstance";
	import type { Context } from "$lib/Context";
	import type { DeviceInfo } from "$lib/DeviceInfo";
	import type { Profile } from "$lib/Profile";
	import type { CopiedItem } from "$lib/propertyInspector";

	import Key from "./Key.svelte";

	import { t } from "$lib/i18n";
	import { inspectedInstance, inspectedParentAction } from "$lib/propertyInspector";
	import { profileManager } from "$lib/singletons";
	import { listen } from "@tauri-apps/api/event";
	import { onDestroy } from "svelte";

	import { invoke } from "@tauri-apps/api/core";

	export let device: DeviceInfo;
	export let profile: Profile;

	export let selectedDevice: string;

	function handleDragStart({ dataTransfer }: DragEvent, controller: string, position: number) {
		if (!dataTransfer) return;
		dataTransfer.effectAllowed = "move";
		dataTransfer.setData("controller", controller);
		dataTransfer.setData("position", position.toString());
	}

	function handleDragOver(event: DragEvent) {
		event.preventDefault();
		if (!event.dataTransfer) return;
		if (event.dataTransfer.types.includes("action")) event.dataTransfer.dropEffect = "copy";
		else if (event.dataTransfer.types.includes("controller")) event.dataTransfer.dropEffect = "move";
	}

	async function handleDrop({ dataTransfer }: DragEvent, controller: string, position: number) {
		let context = { device: device.id, profile: profile.id, controller, position };
		let array = controller == "Encoder" ? profile.sliders : controller == "Infobar" ? profile.infobars : profile.keys;
		if (dataTransfer?.getData("action")) {
			let action = JSON.parse(dataTransfer?.getData("action"));
			if (array[position]) {
				return;
			}
			array[position] = await invoke("create_instance", { context, action });
			profile = profile;
		} else if (dataTransfer?.getData("controller")) {
			let oldController = dataTransfer?.getData("controller");
			let oldArray = oldController == "Encoder" ? profile.sliders : oldController == "Infobar" ? profile.infobars : profile.keys;
			let oldPosition = parseInt(dataTransfer?.getData("position"));
			let response: ActionInstance = await invoke("move_instance", {
				source: { device: device.id, profile: profile.id, controller: oldController, position: oldPosition },
				destination: context,
				retain: false,
			});
			if (response) {
				array[position] = response;
				oldArray[oldPosition] = null;
				profile = profile;
			}
		}
	}

	async function handlePaste(item: CopiedItem, destination: Context) {
		let array = destination.controller == "Encoder" ? profile.sliders : destination.controller == "Infobar" ? profile.infobars : profile.keys;

		if (item.type == "action") {
			if (array[destination.position]) return;
			array[destination.position] = await invoke("create_instance", { context: destination, action: item.action });
			profile = profile;
			return;
		}

		let response: ActionInstance = await invoke("move_instance", { source: item.source, destination, retain: true });
		if (response) {
			array[destination.position] = response;
			profile = profile;
		}
	}

	// Orvo: NineDeck shows its slider as a tall bar beside the keys, moving live with the real one.
	$: isNineDeck = device.id.startsWith("nd-");
	let sliderValue: number | null = null;

	// Slider lock: locked (default) = the slider does the same thing on every page.
	let sliderLocked = true;
	$: sliderLockKey = `orvo-slider-lock-${device.id}`;
	$: try {
		sliderLocked = localStorage.getItem(sliderLockKey) != "0";
	} catch {
		sliderLocked = true;
	}
	function toggleSliderLock() {
		sliderLocked = !sliderLocked;
		try {
			localStorage.setItem(sliderLockKey, sliderLocked ? "1" : "0");
		} catch {}
		invoke("orvo_slider_lock", { device: device.id, profile: profile.id, locked: sliderLocked });
	}
	let lastSliderSync = "";
	$: if (isNineDeck && profile && device.encoders > 0) {
		const key = `${sliderLocked}|${profile.id}|${profile.sliders[0]?.action?.uuid ?? ""}`;
		if (key != lastSliderSync) {
			lastSliderSync = key;
			invoke("orvo_slider_lock", { device: device.id, profile: profile.id, locked: sliderLocked });
		}
	}
	const unlistenSlider = listen("pad_slider", ({ payload }: { payload: { device: string; value: number } }) => {
		if (payload.device == device.id) sliderValue = payload.value;
	});
	onDestroy(() => unlistenSlider.then((f) => f()));

	// Page bar under the pad: the same ◀ ▶ the pad's page keys use.
	let pages: string[] = [];
	async function loadPages(_id: string) {
		try {
			pages = ((await invoke("get_profiles", { device: device.id })) as string[]).sort((a, b) => a.localeCompare(b));
		} catch {
			pages = [];
		}
	}
	$: loadPages(profile.id);
	$: pageIndex = Math.max(0, pages.indexOf(profile.id));
	const pageName = (id: string) => (id?.includes("/") ? id.split("/")[1] : id);
	function flip(step: number) {
		if (pages.length < 2) return;
		$profileManager?.setProfile(pages[(pageIndex + step + pages.length) % pages.length]);
	}

	$: overflowsX = Math.max(device.columns, device.encoders, device.touchpoints) > 8;
	$: overflowsY = device.rows + Math.min(device.encoders, 1) + Math.min(device.touchpoints, 1) > 4;

	// Grid navigation: track focused cell and compute row lengths for arrow key movement.
	let focusedRow = 0;
	let focusedCol = 0;

	$: gridRowLengths = [
		...Array(device.rows).fill(device.columns),
		...(device.encoders > 0 ? [device.encoders] : []),
		...(device.touchpoints > 0 || device.infobars > 0 ? [device.touchpoints + device.infobars] : []),
	];
	$: encoderRowIndex = device.rows;
	$: touchpointRowIndex = device.rows + (device.encoders > 0 ? 1 : 0);
	$: keypadRowWidth = device.columns * 132;

	function flatIndexFromRowCol(row: number, col: number): number {
		let index = 0;
		for (let r = 0; r < row; r++) index += gridRowLengths[r];
		return index + col;
	}

	function rowColFromFlatIndex(flatIndex: number): [number, number] {
		let remaining = flatIndex;
		for (let r = 0; r < gridRowLengths.length; r++) {
			if (remaining < gridRowLengths[r]) return [r, remaining];
			remaining -= gridRowLengths[r];
		}
		return [0, 0];
	}

	function handleGridKeydown(event: KeyboardEvent) {
		const target = event.target as HTMLElement;
		if (target.getAttribute("role") !== "gridcell") return;
		if (!["ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) return;

		event.preventDefault();
		event.stopPropagation();

		let newRow = focusedRow;
		let newCol = focusedCol;

		switch (event.key) {
			case "ArrowRight":
				newCol = Math.min(focusedCol + 1, gridRowLengths[focusedRow] - 1);
				break;
			case "ArrowLeft":
				newCol = Math.max(focusedCol - 1, 0);
				break;
			case "ArrowDown":
				newRow = Math.min(focusedRow + 1, gridRowLengths.length - 1);
				newCol = Math.min(focusedCol, gridRowLengths[newRow] - 1);
				break;
			case "ArrowUp":
				newRow = Math.max(focusedRow - 1, 0);
				newCol = Math.min(focusedCol, gridRowLengths[newRow] - 1);
				break;
			case "Home":
				newCol = 0;
				break;
			case "End":
				newCol = gridRowLengths[focusedRow] - 1;
				break;
		}

		if (newRow === focusedRow && newCol === focusedCol) return;

		focusedRow = newRow;
		focusedCol = newCol;

		const grid = event.currentTarget as HTMLElement;
		const cells = grid.querySelectorAll("[role='gridcell']");
		(cells[flatIndexFromRowCol(newRow, newCol)] as HTMLElement)?.focus();
	}

	function handleGridFocusin(event: FocusEvent) {
		const grid = event.currentTarget as HTMLElement;
		const cells = Array.from(grid.querySelectorAll("[role='gridcell']"));
		const index = cells.indexOf(event.target as Element);
		if (index === -1) return;
		[focusedRow, focusedCol] = rowColFromFlatIndex(index);
	}
</script>

{#key device}
	<span id="grid-description" class="sr-only">{$t("device_view.grid_description")}</span>
	<div
		class="flex flex-col justify-center grow px-10 py-6 overflow-auto"
		class:items-center={device.columns <= 9}
		class:hidden={$inspectedParentAction || selectedDevice != device.id}
		class:device-fade-x={overflowsX && !overflowsY}
		class:device-fade-y={overflowsY && !overflowsX}
		class:device-fade-xy={overflowsX && overflowsY}
		role="grid"
		aria-label={device.name}
		aria-describedby="grid-description"
		tabindex="-1"
		on:click={() => inspectedInstance.set(null)}
		on:keyup={() => inspectedInstance.set(null)}
		on:keydown|capture={handleGridKeydown}
		on:focusin={handleGridFocusin}
	>
		<div class="flex items-baseline gap-3 mb-5 flex-wrap" style={`max-width: ${Math.max(keypadRowWidth, 440) + 48}px; width: 100%`}>
			<h1 class="m-0 text-2xl font-bold tracking-tight text-neutral-100">{pageName(profile.id)}</h1>
			<span class="text-sm text-neutral-400">Page {pageIndex + 1} of {Math.max(pages.length, 1)} · click a box to edit it</span>
		</div>
		<div class="p-5 rounded-3xl bg-neutral-900 border border-neutral-700" style="box-shadow: var(--orvo-shadow)">
		<div class="flex flex-row items-start gap-6">
		<div class="flex flex-col" role="rowgroup">
			{#each { length: device.rows } as _, r}
				<div class="flex flex-row" role="row">
					{#each { length: device.columns } as _, c}
						<Key
							context={{ device: device.id, profile: profile.id, controller: "Keypad", position: r * device.columns + c }}
							bind:inslot={profile.keys[r * device.columns + c]}
							on:dragover={handleDragOver}
							on:drop={(event) => handleDrop(event, "Keypad", r * device.columns + c)}
							on:dragstart={(event) => handleDragStart(event, "Keypad", r * device.columns + c)}
							{handlePaste}
							size={device.id.startsWith("sd-") && device.rows == 4 && device.columns == 8 ? 192 : 144}
							label="{$t('device_view.key')} {String.fromCharCode(65 + r)}{c + 1}"
							tabindex={focusedRow === r && focusedCol === c ? 0 : -1}
						/>
					{/each}
				</div>
			{/each}
		</div>

		{#if isNineDeck && device.encoders > 0}
			<div class="flex flex-col gap-3" role="row" aria-label="Sliders">
				{#each { length: device.encoders } as _, i}
					<div
						class="relative flex flex-col items-center justify-center w-[148px] rounded-2xl border border-neutral-600 bg-neutral-800 overflow-hidden"
						style={`height: ${Math.round(2.5 * 132)}px`}
						role="gridcell"
						aria-label="Slider {i + 1}{sliderValue != null ? `, ${Math.round(sliderValue / 10)}%` : ''}"
					>
						<div class="absolute left-0 right-0 bottom-0 transition-[height] duration-75" style={`height: ${(sliderValue ?? 0) / 10}%; background: var(--orvo-accent); opacity: 0.28`}></div>
						<div class="absolute left-1/2 -translate-x-1/2 top-3 bottom-3 w-1.5 rounded-full bg-neutral-600"></div>
						<span class="absolute top-2 left-0 right-0 text-center text-xs font-semibold text-neutral-200 tabular-nums">{sliderValue != null ? `${Math.round(sliderValue / 10)}%` : "Slider"}</span>
						<button
							class="absolute top-1.5 right-1.5 z-10 p-1 rounded-md hover:bg-neutral-700 {sliderLocked ? 'text-neutral-200' : 'text-neutral-500'}"
							title={sliderLocked ? "Locked: same on every page. Click to set it per page." : "Per page: each page has its own slider action. Click to lock it to all pages."}
							aria-label={sliderLocked ? "Slider locked to all pages" : "Slider set per page"}
							aria-pressed={sliderLocked}
							on:click|stopPropagation={toggleSliderLock}
						>
							<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round">
								<rect x="4" y="11" width="16" height="10" rx="2" />
								{#if sliderLocked}<path d="M8 11V7a4 4 0 0 1 8 0v4" />{:else}<path d="M8 11V7a4 4 0 0 1 7.5-2" />{/if}
							</svg>
						</button>
						<Key
							context={{ device: device.id, profile: profile.id, controller: "Encoder", position: i }}
							bind:inslot={profile.sliders[i]}
							on:dragover={handleDragOver}
							on:drop={(event) => handleDrop(event, "Encoder", i)}
							on:dragstart={(event) => handleDragStart(event, "Encoder", i)}
							{handlePaste}
							square={true}
							label="Slider {i + 1}"
							tabindex={-1}
						/>
					</div>
				{/each}
			</div>
		{/if}
		</div>

		<div class="flex flex-row justify-between" role="row" style={`width: ${keypadRowWidth}px;`} class:hidden={isNineDeck}>
			{#each { length: device.encoders } as _, i}
				<Key
					context={{ device: device.id, profile: profile.id, controller: "Encoder", position: i }}
					bind:inslot={profile.sliders[i]}
					on:dragover={handleDragOver}
					on:drop={(event) => handleDrop(event, "Encoder", i)}
					on:dragstart={(event) => handleDragStart(event, "Encoder", i)}
					{handlePaste}
					size={device.id.startsWith("sd-") && device.rows == 4 && device.columns == 8 ? 192 : 144}
					label="{$t('device_view.encoder')} {i + 1}"
					tabindex={focusedRow === encoderRowIndex && focusedCol === i ? 0 : -1}
				/>
			{/each}
		</div>

		<div class="flex flex-row items-center" role="row">
			{#each { length: device.touchpoints } as _, i}
				<!-- On the Stream Deck Neo, the infobar display sits physically between the two touchpoints. -->
				{#if device.infobars > 0 && i === 1}
					{#each { length: device.infobars } as _, j}
						<div class="px-3.5 py-[3.5px]">
							<Key
								context={{ device: device.id, profile: profile.id, controller: "Infobar", position: j }}
								bind:inslot={profile.infobars[j]}
								on:dragover={handleDragOver}
								on:drop={(event) => handleDrop(event, "Infobar", j)}
								on:dragstart={(event) => handleDragStart(event, "Infobar", j)}
								{handlePaste}
								size={device.id.startsWith("sd-") && device.rows == 4 && device.columns == 8 ? 192 : 144}
								width={248}
								height={58}
							/>
						</div>
					{/each}
				{/if}
				<Key
					context={{ device: device.id, profile: profile.id, controller: "Keypad", position: device.rows * device.columns + i }}
					bind:inslot={profile.keys[device.rows * device.columns + i]}
					on:dragover={handleDragOver}
					on:drop={(event) => handleDrop(event, "Keypad", device.rows * device.columns + i)}
					on:dragstart={(event) => handleDragStart(event, "Keypad", device.rows * device.columns + i)}
					{handlePaste}
					size={device.id.startsWith("sd-") && device.rows == 4 && device.columns == 8 ? 192 : 144}
					isTouchPoint
					label="{$t('device_view.touchpoint')} {i + 1}"
					tabindex={focusedRow === touchpointRowIndex && focusedCol === i ? 0 : -1}
				/>
			{/each}
		</div>
		</div>

		{#if pages.length > 0}
			<div class="flex items-center justify-center gap-3 mt-5" role="group" aria-label="Page keys">
				<button
					class="flex items-center justify-center w-14 h-12 rounded-xl bg-neutral-900 border border-neutral-700 text-neutral-200 hover:border-blue-500 transition-colors"
					aria-label="Previous page"
					on:click|stopPropagation={() => flip(-1)}>◀</button
				>
				<span class="min-w-40 text-center text-sm text-neutral-300"><span class="font-semibold text-neutral-100">{pageName(profile.id)}</span> · {pageIndex + 1}/{pages.length}</span>
				<button
					class="flex items-center justify-center w-14 h-12 rounded-xl bg-neutral-900 border border-neutral-700 text-neutral-200 hover:border-blue-500 transition-colors"
					aria-label="Next page"
					on:click|stopPropagation={() => flip(1)}>▶</button
				>
			</div>
		{/if}
	</div>
{/key}

<style>
	.device-fade-x {
		mask-image: linear-gradient(to right, transparent, black 7.5rem, black calc(100% - 7.5rem), transparent);
	}
	.device-fade-y {
		mask-image: linear-gradient(to bottom, transparent, black 7.5rem, black calc(100% - 7.5rem), transparent);
	}
	.device-fade-xy {
		mask-image:
			linear-gradient(to right, transparent, black 7.5rem, black calc(100% - 7.5rem), transparent),
			linear-gradient(to bottom, transparent, black 7.5rem, black calc(100% - 7.5rem), transparent);
		mask-composite: intersect;
	}
</style>
