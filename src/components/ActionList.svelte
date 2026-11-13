<script lang="ts">
	import type { Action } from "$lib/Action";

	import MagnifyingGlass from "phosphor-svelte/lib/MagnifyingGlass";

	import { t } from "$lib/i18n";
	import { getWebserverUrl } from "$lib/ports";
	import { copiedItem } from "$lib/propertyInspector";
	import { localisations } from "$lib/settings";
	import { PRODUCT_NAME } from "$lib/singletons";

	import { invoke } from "@tauri-apps/api/core";

	let categories: { [name: string]: { icon?: string; actions: Action[] } } = {};
	let plugins: any[] = [];
	export async function reload() {
		categories = await invoke("get_categories");
		plugins = await invoke("list_plugins");
	}
	reload();

	let query: string = "";
	let filteredCategories: [string, { icon?: string; actions: Action[] }][] = [];
	$: {
		let lowerCaseQuery = query.toLowerCase().trim();
		filteredCategories = Object.entries(categories)
			.sort((a, b) => (a[0] == PRODUCT_NAME ? -1 : b[0] == PRODUCT_NAME ? 1 : a[0].localeCompare(b[0])))
			.map(([categoryName, { icon, actions }]): [string, { icon?: string; actions: Action[] }] => {
				if (!categoryName.toLowerCase().includes(lowerCaseQuery)) {
					actions = actions.filter((action) => action.name.toLowerCase().includes(lowerCaseQuery));
				}
				return [categoryName, { icon, actions }];
			})
			.filter(([_, { actions }]) => actions.length > 0);
	}

	function handleListKeydown(event: KeyboardEvent) {
		if (!["ArrowUp", "ArrowDown", "Home", "End"].includes(event.key)) return;
		const list = event.currentTarget as HTMLElement;
		const items = Array.from(list.querySelectorAll("[role='option']"));
		const currentIndex = items.indexOf(event.target as Element);
		if (currentIndex == -1) return;

		event.preventDefault();

		let newIndex = currentIndex;
		switch (event.key) {
			case "ArrowDown":
				newIndex = Math.min(currentIndex + 1, items.length - 1);
				break;
			case "ArrowUp":
				newIndex = Math.max(currentIndex - 1, 0);
				break;
			case "Home":
				newIndex = 0;
				break;
			case "End":
				newIndex = items.length - 1;
				break;
		}

		if (newIndex == currentIndex) return;
		(items[currentIndex] as HTMLElement).tabIndex = -1;
		(items[newIndex] as HTMLElement).tabIndex = 0;
		(items[newIndex] as HTMLElement).focus();
	}

	function handleListFocusin(event: FocusEvent) {
		const list = event.currentTarget as HTMLElement;
		const items = Array.from(list.querySelectorAll("[role='option']"));
		const index = items.indexOf(event.target as Element);
		if (index == -1) return;
		for (let i = 0; i < items.length; i++) {
			(items[i] as HTMLElement).tabIndex = i == index ? 0 : -1;
		}
	}

	// Remember which categories are folded shut.
	let closedCategories: string[] = [];
	try {
		closedCategories = JSON.parse(localStorage.getItem("orvo-closed-categories") ?? "[]");
	} catch {}
	function rememberCategory(name: string, open: boolean) {
		const has = closedCategories.includes(name);
		if (open == !has) return;
		closedCategories = open ? closedCategories.filter((c) => c != name) : [...closedCategories, name];
		try {
			localStorage.setItem("orvo-closed-categories", JSON.stringify(closedCategories));
		} catch {}
	}
</script>

<div class="flex flex-col grow min-h-0">
	<div class="flex flex-row items-center mx-3 mb-2 h-10 bg-neutral-800 border border-neutral-700 rounded-xl">
		<MagnifyingGlass size="13" class="ml-2 mr-1 text-neutral-300" />
		<input bind:value={query} class="w-full p-1 text-sm text-neutral-200 bg-transparent" placeholder={$t("action_list.search_placeholder")} type="search" spellcheck="false" />
	</div>

	<span id="action-list-hint" class="sr-only">{$t("action_list.hint")}</span>
	<div class="grow overflow-auto select-none px-2 pb-3">
		{#each filteredCategories as [name, { icon, actions }]}
			<details open={!closedCategories.includes(name)} on:toggle={(e) => rememberCategory(name, e.currentTarget.open)}>
				<summary class="px-2 py-2.5 text-sm font-semibold text-neutral-200 hover:bg-neutral-800 rounded-lg transition-colors cursor-pointer">
					{#if icon || (actions[0] && plugins.find((x) => x.id == actions[0].plugin) && categories[name].actions.every((x) => x.plugin == actions[0].plugin))}
						<img
							src={icon
								? !icon.startsWith("opendeck/")
									? getWebserverUrl(icon)
									: icon.replace("opendeck", "")
								: getWebserverUrl(plugins.find((x) => x.id == actions[0].plugin).icon)}
							alt={name}
							class="w-5 h-5 rounded-xs ml-1 -mt-1 inline"
						/>
					{/if}
					<span class="ml-1">{name}</span>
				</summary>
				<div role="listbox" aria-label={name} aria-describedby="action-list-hint" tabindex="-1" on:keydown={handleListKeydown} on:focusin={handleListFocusin}>
					{#each actions as action, i}
						<div
							class="flex flex-row items-center p-1.5 pl-4 rounded-lg hover:bg-neutral-800 transition-colors cursor-grab active:cursor-grabbing"
							draggable="true"
							title={$localisations?.[action.plugin]?.[action.uuid]?.Tooltip ?? action.tooltip}
							role="option"
							aria-selected="false"
							tabindex={i == 0 ? 0 : -1}
							aria-label={$localisations?.[action.plugin]?.[action.uuid]?.Name ?? action.name}
							on:dragstart={(event) => {
								if (!event.dataTransfer) return;
								event.dataTransfer.effectAllowed = "copy";
								event.dataTransfer.setData("action", JSON.stringify(action));
							}}
							on:keydown={(event) => {
								if ((event.ctrlKey || event.metaKey) && event.key == "c") {
									copiedItem.set({ type: "action", action });
								}
							}}
						>
							<img
								src={!action.icon.startsWith("opendeck/") ? getWebserverUrl(action.icon) : action.icon.replace("opendeck", "")}
								alt=""
								class="m-0.5 mr-3 w-9 h-9 rounded-lg border border-neutral-700 pointer-events-none"
							/>
							<span class="text-sm text-neutral-300">{$localisations?.[action.plugin]?.[action.uuid]?.Name ?? action.name}</span>
						</div>
					{/each}
				</div>
			</details>
		{/each}
	</div>
</div>
