<script lang="ts">
	import type { ActionInstance } from "$lib/ActionInstance";
	import type { Context } from "$lib/Context";
	import type { DeviceInfo } from "$lib/DeviceInfo";
	import type { Profile } from "$lib/Profile";
	import type { CopiedItem } from "$lib/propertyInspector";

	import Key from "./Key.svelte";

	import { t } from "$lib/i18n";
	import { inspectedInstance, inspectedParentAction, placeAction } from "$lib/propertyInspector";
	import { profileManager } from "$lib/singletons";
	import { listen } from "@tauri-apps/api/event";
	import { onDestroy, tick } from "svelte";

	import { invoke } from "@tauri-apps/api/core";
	import { finishSoundKey, plainAction, type SoundAction } from "$lib/soundboard";

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
			let action: SoundAction = JSON.parse(dataTransfer?.getData("action"));
			if (array[position]) {
				return;
			}
			array[position] = await finishSoundKey(await invoke("create_instance", { context, action: plainAction(action) }), action);
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
			array[destination.position] = await finishSoundKey(await invoke("create_instance", { context: destination, action: plainAction(item.action) }), item.action);
			profile = profile;
			return;
		}

		let response: ActionInstance = await invoke("move_instance", { source: item.source, destination, retain: true });
		if (response) {
			array[destination.position] = response;
			profile = profile;
		}
	}

	// A click only clears the selection when it also started on the background. Opening the action
	// tray mid-click shrinks the pad, so the release can land beside the box that was pressed.
	let pressStartedOnBox = false;
	function notePress(event: PointerEvent) {
		pressStartedOnBox = !!(event.target as HTMLElement | null)?.closest?.("canvas, button");
	}
	function clearOnBackground() {
		if (!pressStartedOnBox) inspectedInstance.set(null);
		pressStartedOnBox = false;
	}

	// Orvo: the action tray puts an action on the selected empty box (click a box, then click an action).
	$: if ($placeAction && $placeAction.context.device == device.id && $placeAction.context.profile == profile.id) {
		const { context, action } = $placeAction;
		placeAction.set(null);
		handlePaste({ type: "action", action }, context).then(async () => {
			await tick();
			const array = context.controller == "Encoder" ? profile.sliders : context.controller == "Infobar" ? profile.infobars : profile.keys;
			if (array[context.position]) inspectedInstance.set(`${context.device}.${context.profile}.${context.controller}.${context.position}.0`);
		});
	}

	// Orvo: NineDeck shows its slider as a tall bar beside the keys, moving live with the real one.
	$: isNineDeck = device.id.startsWith("nd-");
	let sliderValue: number | null = null;

	// Slider lock: locked (default) = one slider action on top of every page; each page's own
	// slider action is kept underneath and comes back when unlocked.
	let sliderLocked = true;
	let lockDevice = "";
	$: if (isNineDeck && device.id != lockDevice) {
		lockDevice = device.id;
		invoke("orvo_slider_locked", { device: device.id }).then((v) => (sliderLocked = v as boolean)).catch(() => {});
	}
	async function reloadProfile() {
		try {
			const fresh: Profile = await invoke("get_selected_profile", { device: device.id });
			if (fresh.id == profile.id) {
				profile = fresh;
				lastSliderSync = `${profile.id}|${sliderSig(profile)}`;
			}
		} catch {}
	}
	async function toggleSliderLock() {
		sliderLocked = !sliderLocked;
		await invoke("orvo_slider_lock", { device: device.id, profile: profile.id, locked: sliderLocked });
		await reloadProfile();
	}
	const sliderSig = (p: Profile) => {
		const s = p.sliders?.[0];
		return s ? `${s.action.uuid}` : "";
	};
	let lastSliderSync = "";
	$: if (isNineDeck && profile && device.encoders > 0) {
		const pageId = profile.id;
		const key = `${pageId}|${sliderSig(profile)}`;
		if (key != lastSliderSync) {
			const samePage = lastSliderSync.startsWith(pageId + "|");
			lastSliderSync = key;
			if (samePage) invoke("orvo_slider_changed", { device: device.id, profile: pageId }).catch(() => {});
			else invoke("orvo_slider_refresh", { device: device.id }).then(reloadProfile).catch(() => {});
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

	// Orvo: the action tray sits under the pad, so shrink the pad to fit the room that is left.
	let availableHeight = 0;
	let availableWidth = 0;
	$: naturalWidth = 40 + device.columns * 132 + (isNineDeck ? 24 + 308 : 0);
	$: naturalHeight =
		52 + 40 + (isNineDeck ? Math.max(device.rows * 132, 396) : device.rows * 132 + (device.encoders > 0 ? 132 : 0) + (pages.length > 0 ? 68 : 0)) + (device.touchpoints > 0 ? 132 : 0);
	$: padZoom =
		availableHeight > 0 && availableWidth > 0 && !overflowsX && !overflowsY
			? Math.max(0.5, Math.min(1, (availableHeight - 40) / naturalHeight, (availableWidth - 56) / naturalWidth))
			: 1;

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
		class="flex flex-col justify-center grow min-h-0 px-6 py-4 overflow-auto"
		bind:clientHeight={availableHeight}
		bind:clientWidth={availableWidth}
		class:items-center={device.columns <= 9}
		class:hidden={$inspectedParentAction || selectedDevice != device.id}
		class:device-fade-x={overflowsX && !overflowsY}
		class:device-fade-y={overflowsY && !overflowsX}
		class:device-fade-xy={overflowsX && overflowsY}
		role="grid"
		aria-label={device.name}
		aria-describedby="grid-description"
		tabindex="-1"
		on:pointerdown|capture={notePress}
		on:click={clearOnBackground}
		on:keyup={() => inspectedInstance.set(null)}
		on:keydown|capture={handleGridKeydown}
		on:focusin={handleGridFocusin}
	>
		<div class="flex flex-col" class:items-center={device.columns <= 9} style={`zoom: ${padZoom}; --orvo-zoom: ${padZoom}`}>
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

		{#if isNineDeck}
			<!-- Same layout as the pad's screen: page keys at the top, the slider's level bar under ◀,
			     the space under ▶ kept for a second slider. -->
			<div class="flex flex-row gap-3" aria-label="Page keys and slider">
				<div class="flex flex-col gap-3 w-[148px]">
					<button
						class="flex items-center justify-center h-[128px] rounded-2xl bg-neutral-800 border border-neutral-600 text-3xl text-neutral-200 hover:border-blue-500 transition-colors"
						aria-label="Previous page"
						title="Previous page"
						on:click|stopPropagation={() => flip(-1)}>◀</button
					>
					{#each { length: Math.min(device.encoders, 1) } as _, i}
						<div
							class="relative flex flex-col items-center justify-center rounded-2xl border border-neutral-600 bg-neutral-800 overflow-hidden"
							style="height: 256px"
							role="gridcell"
							aria-label="Slider {i + 1}{sliderValue != null ? `, ${Math.round(sliderValue / 10)}%` : ''}"
						>
							<div class="absolute left-0 right-0 bottom-0 transition-[height] duration-75" style={`height: ${(sliderValue ?? 0) / 10}%; background: var(--orvo-accent); opacity: 0.28`}></div>
							<span class="absolute top-2 left-0 right-0 text-center text-lg font-bold text-neutral-100 tabular-nums">{sliderValue != null ? Math.round(sliderValue / 10) : "–"}</span>
							<button
								class="absolute top-1.5 right-1.5 z-10 p-1 rounded-md hover:bg-neutral-700 {sliderLocked ? 'text-neutral-200' : 'text-neutral-500'}"
								title={sliderLocked ? "Locked: this slider action is on top of every page. Click to unlock (each page uses its own)." : "Unlocked: each page has its own slider action. Click to lock one on top of all pages."}
								aria-label={sliderLocked ? "Slider locked to all pages" : "Slider set per page"}
								aria-pressed={sliderLocked}
								on:click|stopPropagation={toggleSliderLock}
							>
								<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round">
									<rect x="4" y="11" width="16" height="10" rx="2" />
									{#if sliderLocked}<path d="M8 11V7a4 4 0 0 1 8 0v4" />{:else}<path d="M8 11V7a4 4 0 0 1 7.5-2" />{/if}
								</svg>
							</button>
							<div class="mt-6">
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
						</div>
					{/each}
				</div>
				<div class="flex flex-col gap-3 w-[148px]">
					<button
						class="flex items-center justify-center h-[128px] rounded-2xl bg-neutral-800 border border-neutral-600 text-3xl text-neutral-200 hover:border-blue-500 transition-colors"
						aria-label="Next page"
						title="Next page"
						on:click|stopPropagation={() => flip(1)}>▶</button
					>
					<div class="rounded-2xl border border-dashed border-neutral-700" style="height: 256px" title="Room for a second slider"></div>
				</div>
			</div>
		{/if}
		</div>

		{#if !isNineDeck}
		<div class="flex flex-row justify-between" role="row" style={`width: ${keypadRowWidth}px;`}>
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
		{/if}

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

		{#if pages.length > 0 && !isNineDeck}
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
