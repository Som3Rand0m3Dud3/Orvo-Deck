<script lang="ts">
	import type { Action } from "$lib/Action";
	import type { Context } from "$lib/Context";

	import CaretDown from "phosphor-svelte/lib/CaretDown";
	import CaretUp from "phosphor-svelte/lib/CaretUp";
	import MagnifyingGlass from "phosphor-svelte/lib/MagnifyingGlass";

	import { t } from "$lib/i18n";
	import { getWebserverUrl } from "$lib/ports";
	import { copiedItem, inspectedInstance, placeAction } from "$lib/propertyInspector";
	import { localisations } from "$lib/settings";
	import { PRODUCT_NAME } from "$lib/singletons";

	import { trayScale } from "$lib/tray";

	import { invoke } from "@tauri-apps/api/core";
	import { tick } from "svelte";
	import { cubicOut } from "svelte/easing";
	import { fly, slide, type TransitionConfig } from "svelte/transition";

	// Orvo: the action tray under the keys. Category tabs on one scrolling row, two rows of
	// action cubes, and an arrow that tucks the whole tray away.

	type Category = { icon?: string; actions: Action[] };
	let categories: { [name: string]: Category } = {};
	let plugins: any[] = [];
	export async function reload() {
		categories = await invoke("get_categories");
		plugins = await invoke("list_plugins");
	}
	reload();

	let sorted: [string, Category][] = [];
	$: sorted = Object.entries(categories)
		.filter(([_, { actions }]) => actions.length > 0)
		.sort((a, b) => (a[0] == PRODUCT_NAME ? -1 : b[0] == PRODUCT_NAME ? 1 : a[0].localeCompare(b[0])));

	const read = (key: string) => {
		try {
			return localStorage.getItem(key);
		} catch {
			return null;
		}
	};
	const write = (key: string, value: string) => {
		try {
			localStorage.setItem(key, value);
		} catch {}
	};

	let selected = read("orvo-tray-tab") ?? "";
	$: if (sorted.length > 0 && !sorted.some(([name]) => name == selected)) selected = sorted[0][0];
	// Cubes slide toward the tab you picked: a tab to the right pushes them left, and the other way.
	let dir = 0;
	function selectTab(name: string, event: Event) {
		const from = query.trim() ? -1 : sorted.findIndex(([n]) => n == selected);
		const to = sorted.findIndex(([n]) => n == name);
		dir = from < 0 || to < 0 ? 0 : Math.sign(to - from);
		selected = name;
		query = "";
		write("orvo-tray-tab", name);
		(event.currentTarget as HTMLElement).scrollIntoView({ inline: "nearest", block: "nearest" });
	}

	$: nameOf = (action: Action): string => $localisations?.[action.plugin]?.[action.uuid]?.Name ?? action.name;

	let query = "";
	let lastSearching = false;
	$: if (!!query.trim() != lastSearching) {
		lastSearching = !!query.trim();
		dir = 0;
	}
	$: gridKey = query.trim() ? "\u0000search" : selected;
	const SLIDE = { duration: 220, easing: cubicOut, opacity: 1 };
	function slideIn(node: Element): TransitionConfig {
		return dir ? fly(node, { ...SLIDE, x: dir * (node.parentElement?.clientWidth ?? 600) }) : { duration: 0 };
	}
	function slideOut(node: Element): TransitionConfig {
		return dir ? fly(node, { ...SLIDE, x: -dir * (node.parentElement?.clientWidth ?? 600) }) : { duration: 0 };
	}

	let shown: Action[] = [];
	$: {
		const lower = query.toLowerCase().trim();
		shown = lower
			? sorted.flatMap(([name, { actions }]) =>
					name.toLowerCase().includes(lower) ? actions : actions.filter((action) => nameOf(action).toLowerCase().includes(lower)),
				)
			: (categories[selected]?.actions ?? []);
	}

	function categoryIcon(name: string, category: Category, plugins: any[]): string | null {
		if (category.icon) return !category.icon.startsWith("opendeck/") ? getWebserverUrl(category.icon) : category.icon.replace("opendeck", "");
		const first = category.actions[0];
		const plugin = first && plugins.find((x) => x.id == first.plugin);
		if (plugin && category.actions.every((x) => x.plugin == first.plugin)) return getWebserverUrl(plugin.icon);
		return null;
	}
	const actionIcon = (action: Action) => (!action.icon.startsWith("opendeck/") ? getWebserverUrl(action.icon) : action.icon.replace("opendeck", ""));

	// Minimized: only the arrow shows. The arrow, or clicking an empty box, pulls the tray back up.
	let minimized = read("orvo-tray-min") == "1";

	// Height of the cube area (screen pixels): three rows by default, drag the top edge to change it.
	const ROW = 86;
	const GAP = 6;
	const rowsHeight = (rows: number, scale: number) => (rows * ROW + (rows - 1) * GAP) * scale;
	let savedHeight = parseFloat(read("orvo-tray-height") ?? "");
	let gridHeight = savedHeight > 0 ? savedHeight : rowsHeight(3, $trayScale);
	let windowHeight = globalThis.innerHeight ?? 800;
	$: minHeight = rowsHeight(1, $trayScale);
	$: maxHeight = Math.max(minHeight, windowHeight * 0.55);
	$: shownHeight = Math.min(maxHeight, Math.max(minHeight, gridHeight));
	let resizing = false;
	let startY = 0;
	let startHeight = 0;
	function startResize(event: PointerEvent) {
		if (event.button != 0) return;
		resizing = true;
		startY = event.clientY;
		startHeight = shownHeight;
		(event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
		event.preventDefault();
	}
	function moveResize(event: PointerEvent) {
		if (resizing) gridHeight = Math.min(maxHeight, Math.max(minHeight, startHeight + startY - event.clientY));
	}
	function endResize() {
		if (!resizing) return;
		resizing = false;
		write("orvo-tray-height", String(Math.round(gridHeight)));
	}
	function resizeKey(event: KeyboardEvent) {
		const step = event.key == "ArrowUp" ? 24 : event.key == "ArrowDown" ? -24 : 0;
		if (!step) return;
		event.preventDefault();
		gridHeight = Math.min(maxHeight, Math.max(minHeight, shownHeight + step));
		write("orvo-tray-height", String(Math.round(gridHeight)));
	}
	function setMinimized(value: boolean) {
		minimized = value;
		write("orvo-tray-min", value ? "1" : "0");
	}

	// An empty box is selected: clicking a cube puts that action on it.
	let armed: Context | null = null;
	$: armed = $inspectedInstance && typeof $inspectedInstance == "object" ? $inspectedInstance : null;
	$: if (armed) setMinimized(false);
	$: armedLabel = armed ? (armed.controller == "Keypad" ? `Key ${armed.position + 1}` : armed.controller == "Encoder" ? `Slider ${armed.position + 1}` : armed.controller) : "";
	const fits = (action: Action, target: Context | null) => !target || action.controllers.includes(target.controller);
	function pick(action: Action) {
		if (!armed || !fits(action, armed)) return;
		placeAction.set({ context: armed, action });
	}

	// Tabs never wrap: the row scrolls sideways. Wheel up moves it right, wheel down left.
	let tabsEl: HTMLDivElement;
	let fadeLeft = false;
	let fadeRight = false;
	function updateFade() {
		if (!tabsEl) return;
		fadeLeft = tabsEl.scrollLeft > 2;
		fadeRight = tabsEl.scrollLeft + tabsEl.clientWidth < tabsEl.scrollWidth - 2;
	}
	function tabsWheel(event: WheelEvent) {
		if (!tabsEl || tabsEl.scrollWidth <= tabsEl.clientWidth) return;
		event.preventDefault();
		tabsEl.scrollLeft += Math.abs(event.deltaX) > Math.abs(event.deltaY) ? event.deltaX : -event.deltaY;
		updateFade();
	}
	$: if (sorted && !minimized) tick().then(updateFade);
</script>

<svelte:window
	on:resize={() => {
		windowHeight = globalThis.innerHeight;
		updateFade();
	}}
/>

<div class="relative shrink-0 bg-neutral-900 border-t border-neutral-700 select-none" class:cursor-ns-resize={resizing}>
	{#if !minimized}
		<!-- A focusable separator is the standard resize handle (arrow keys resize too). -->
		<!-- svelte-ignore a11y-no-noninteractive-tabindex a11y-no-noninteractive-element-interactions -->
		<div
			class="absolute left-0 right-0 -top-1.5 h-3 z-[5] cursor-ns-resize"
			role="separator"
			aria-orientation="horizontal"
			aria-label="Resize the actions panel"
			aria-valuemin={Math.round(minHeight)}
			aria-valuemax={Math.round(maxHeight)}
			aria-valuenow={Math.round(shownHeight)}
			tabindex="0"
			title="Drag to resize"
			on:pointerdown={startResize}
			on:pointermove={moveResize}
			on:pointerup={endResize}
			on:pointercancel={endResize}
			on:keydown={resizeKey}
		></div>
	{/if}
	<button
		class="absolute left-1/2 -translate-x-1/2 -top-[26px] z-10 flex items-center justify-center w-[72px] h-[26px] rounded-t-xl bg-neutral-900 border border-b-0 border-neutral-700 text-neutral-300 hover:text-neutral-100 transition-colors"
		aria-label={minimized ? "Show actions" : "Hide actions"}
		title={minimized ? "Show actions" : "Hide actions"}
		aria-expanded={!minimized}
		on:click={() => setMinimized(!minimized)}
	>
		{#if minimized}<CaretUp size="16" weight="bold" />{:else}<CaretDown size="16" weight="bold" />{/if}
	</button>

	{#if !minimized}
		<div transition:slide={{ duration: 200, easing: cubicOut }}>
		<div class="flex flex-col gap-2 p-2.5" style={`zoom: ${$trayScale}`}>
			<div class="flex flex-row items-center gap-2 min-w-0">
				<div
					bind:this={tabsEl}
					class="tray-tabs flex flex-row items-center gap-1.5 grow min-w-0 overflow-x-auto"
					class:fade-left={fadeLeft}
					class:fade-right={fadeRight}
					role="tablist"
					aria-label="Action categories"
					on:wheel|nonpassive={tabsWheel}
					on:scroll={updateFade}
				>
					{#each sorted as [name, category] (name)}
						{@const icon = categoryIcon(name, category, plugins)}
						{@const on = !query.trim() && name == selected}
						<button
							class="flex flex-row items-center gap-2 shrink-0 h-10 px-3.5 rounded-full border text-sm whitespace-nowrap transition-colors {on
								? 'bg-neutral-800 text-neutral-100'
								: 'border-neutral-700 text-neutral-300 hover:bg-neutral-800'}"
							style={on ? "border-color: var(--orvo-accent)" : ""}
							role="tab"
							aria-selected={on}
							on:click={(event) => selectTab(name, event)}
						>
							{#if icon}<img src={icon} alt="" class="w-5 h-5 rounded-xs" />{/if}
							<span>{name}</span>
							<span class="text-xs text-neutral-400">{category.actions.length}</span>
						</button>
					{/each}
				</div>

				{#if armed}
					<span
						class="shrink-0 h-10 px-3.5 flex items-center rounded-full bg-neutral-800 border text-sm text-neutral-100 whitespace-nowrap"
						style="border-color: var(--orvo-accent)"
						title="Click an action below to put it on {armedLabel}"
					>
						Adding to&nbsp;<strong>{armedLabel}</strong>
					</span>
				{/if}

				<label class="tray-search flex flex-row items-center shrink-0 h-10 bg-neutral-800 border border-neutral-700 rounded-full overflow-hidden cursor-text" class:open={query != ""} title={$t("action_list.search_placeholder")}>
					<MagnifyingGlass size="14" class="shrink-0 ml-[12px] mr-1 text-neutral-300" />
					<input bind:value={query} class="w-full p-1 pr-3 text-sm text-neutral-200 bg-transparent" placeholder={$t("action_list.search_placeholder")} type="search" spellcheck="false" />
				</label>
			</div>

			<div class="relative overflow-hidden" style={`height: ${shownHeight / $trayScale}px`}>
			{#key gridKey}
			<div class="tray-grid absolute inset-0 grid gap-1.5 overflow-y-auto" role="listbox" aria-label={query.trim() ? "Search results" : selected} in:slideIn out:slideOut>
				{#each shown as action (action.plugin + action.uuid)}
					<div
						class="flex flex-col items-center justify-center gap-1.5 h-[86px] px-1 rounded-xl bg-neutral-800 border border-neutral-700 hover:border-neutral-500 transition-colors {armed
							? fits(action, armed)
								? 'cursor-pointer'
								: 'opacity-40 cursor-not-allowed'
							: 'cursor-grab active:cursor-grabbing'}"
						draggable="true"
						title={$localisations?.[action.plugin]?.[action.uuid]?.Tooltip ?? action.tooltip}
						role="option"
						aria-selected="false"
						aria-disabled={!fits(action, armed)}
						aria-label={nameOf(action)}
						tabindex="0"
						on:dragstart={(event) => {
							if (!event.dataTransfer) return;
							event.dataTransfer.effectAllowed = "copy";
							event.dataTransfer.setData("action", JSON.stringify(action));
						}}
						on:click={() => pick(action)}
						on:keydown={(event) => {
							if ((event.ctrlKey || event.metaKey) && event.key == "c") copiedItem.set({ type: "action", action });
							else if (event.key == "Enter") pick(action);
						}}
					>
						<img src={actionIcon(action)} alt="" class="w-9 h-9 rounded-lg border border-neutral-700 pointer-events-none" />
						<span class="tray-name text-xs leading-tight text-center text-neutral-200 pointer-events-none">{nameOf(action)}</span>
					</div>
				{:else}
					<p class="col-span-full self-center text-center text-sm text-neutral-400">No actions found.</p>
				{/each}
			</div>
			{/key}
			</div>
		</div>
		</div>
	{/if}
</div>

<style>
	/* Search is a round button until it is used, so the tabs keep the room. */
	.tray-search {
		width: 2.5rem;
		transition: width 0.15s;
	}
	.tray-search:focus-within,
	.tray-search.open {
		width: 11rem;
	}
	.tray-tabs {
		scrollbar-width: none;
	}
	.tray-tabs::-webkit-scrollbar {
		display: none;
	}
	.tray-tabs.fade-left {
		mask-image: linear-gradient(to right, transparent, black 2.5rem);
	}
	.tray-tabs.fade-right {
		mask-image: linear-gradient(to left, transparent, black 2.5rem);
	}
	.tray-tabs.fade-left.fade-right {
		mask-image: linear-gradient(to right, transparent, black 2.5rem, black calc(100% - 2.5rem), transparent);
	}
	/* As many rows of cubes as the panel's height fits; more than that scrolls. */
	.tray-grid {
		grid-template-columns: repeat(auto-fill, minmax(104px, 1fr));
		grid-auto-rows: 86px;
		align-content: start;
	}
	.tray-name {
		display: -webkit-box;
		-webkit-line-clamp: 2;
		line-clamp: 2;
		-webkit-box-orient: vertical;
		overflow: hidden;
	}
</style>
