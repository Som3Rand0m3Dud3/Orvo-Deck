<script lang="ts">
	import ArrowClockwise from "phosphor-svelte/lib/ArrowClockwise";
	import ArrowSquareOut from "phosphor-svelte/lib/ArrowSquareOut";
	import CloudArrowDown from "phosphor-svelte/lib/CloudArrowDown";
	import FileArrowUp from "phosphor-svelte/lib/FileArrowUp";
	import Gear from "phosphor-svelte/lib/Gear";
	import MagnifyingGlass from "phosphor-svelte/lib/MagnifyingGlass";
	import Robot from "phosphor-svelte/lib/Robot";
	import Trash from "phosphor-svelte/lib/Trash";
	import WarningCircle from "phosphor-svelte/lib/WarningCircle";
	import ListedPlugin from "./ListedPlugin.svelte";
	import PluginDetails from "./PluginDetails.svelte";
	import Popup from "./Popup.svelte";
	import Tooltip from "./Tooltip.svelte";

	import { t } from "$lib/i18n";
	import { getWebserverUrl } from "$lib/ports";
	import { localisations, settings } from "$lib/settings";
	import { actionList, deviceSelector, PRODUCT_NAME } from "$lib/singletons";

	import { invoke } from "@tauri-apps/api/core";
	import { listen } from "@tauri-apps/api/event";
	import { onOpenUrl } from "@tauri-apps/plugin-deep-link";
	import { ask, message } from "$lib/dialog";
	import { open } from "@tauri-apps/plugin-dialog";
	import { fly } from "svelte/transition";

	// @ts-expect-error
	const fetch = window.fetchNative ?? window.fetch;

	let showPopup: boolean;
	setInterval(async () => {
		if (showPopup) installed = await invoke("list_plugins");
	}, 1e3);

	async function installPlugin(name: string, url: string | null, file: string | null, fallback_id: string | null, confirm: boolean = true) {
		if (
			confirm &&
			!file &&
			!(await ask($t("plugin_manager.install.prompt"), {
				title: $t("plugin_manager.install.title", { name }),
				okLabel: $t("dialog.yes"),
				cancelLabel: $t("dialog.no"),
			}))
		)
			return;
		installing = { name, stage: file ? "unpack" : "download", done: 0, total: null };
		try {
			await invoke("install_plugin", { url, file, fallback_id });
			installing = null;
			try {
				if ($settings) localisations.set(await invoke("get_localisations", { locale: $settings.language }));
			} catch {}
			message($t("plugin_manager.install.success", { name }), {
				title: $t("plugin_manager.install.success.title", { name }),
				buttons: { ok: $t("dialog.ok") },
			});
			$actionList?.reload();
			installed = await invoke("list_plugins");
		} catch (error: any) {
			installing = null;
			message(error, { title: $t("plugin_manager.install.error", { name }), buttons: { ok: $t("dialog.ok") } });
		}
	}

	// Install progress card: shown from the moment Install is pressed until it finishes.
	type InstallStage = "find" | "download" | "unpack" | "start" | "node";
	let installing: { name: string; stage: InstallStage; done: number; total: number | null } | null = null;
	listen("install_progress", ({ payload }: { payload: { stage: InstallStage; done: number; total: number | null } }) => {
		if (installing) installing = { ...installing, ...payload };
	});
	const mb = (n: number) => (n / 1048576).toFixed(n < 10485760 ? 1 : 0);
	$: installLabel = !installing
		? ""
		: installing.stage == "find"
			? "Finding the download…"
			: installing.stage == "download"
				? installing.total
					? `Downloading… ${mb(installing.done)} / ${mb(installing.total)} MB`
					: installing.done
						? `Downloading… ${mb(installing.done)} MB`
						: "Downloading…"
				: installing.stage == "unpack"
					? "Unpacking…"
					: installing.stage == "node"
						? installing.total
							? `Setting up Node.js (one time)… ${mb(installing.done)} / ${mb(installing.total)} MB`
							: "Setting up Node.js (one time)…"
						: "Starting the plugin…";
	$: installPercent =
		installing && (installing.stage == "download" || installing.stage == "node") && installing.total ? Math.min(100, (installing.done / installing.total) * 100) : null;

	let choices: any[] | undefined;
	let choice: number;
	let finishChoice = (_: unknown) => {};
	let cancelChoice = () => {};
	async function chooseAsset(assets: any[]): Promise<any> {
		choices = assets;
		try {
			await new Promise((resolve, reject) => {
				finishChoice = resolve;
				cancelChoice = reject;
			});
		} catch (e) {
			throw e;
		} finally {
			choices = undefined;
			finishChoice = (_: unknown) => {};
			cancelChoice = () => {};
		}
		return assets[choice];
	}

	let openDetailsView: string | null = null;
	type GitHubPlugin = {
		name: string;
		author: string;
		repository: string;
		download_url: string | undefined;
		flags?: { ai_assisted?: boolean };
		icon?: string;
		version?: string;
	};
	async function installPluginGitHub(id: string, plugin: GitHubPlugin) {
		if (plugin.download_url) {
			await installPlugin(plugin.name, plugin.download_url, null, id);
			return;
		}

		let endpoint = new URL(plugin.repository);
		endpoint.hostname = "api." + endpoint.hostname;
		endpoint.pathname = "/repos" + endpoint.pathname + "/releases";

		let res;
		installing = { name: plugin.name, stage: "find", done: 0, total: null };
		try {
			res = await (await fetch(endpoint)).json();
			installing = null;
		} catch (error: any) {
			installing = null;
			message(error, { title: $t("plugin_manager.install.error", { name: plugin.name }), buttons: { ok: $t("dialog.ok") } });
			return;
		}

		let release = res[0];
		if (release.prerelease && res.find((r: any) => !r.prerelease)) release = res.find((r: any) => !r.prerelease);

		let assets = [];
		for (const asset of release.assets) {
			if (asset.name.toLowerCase().endsWith(".streamdeckplugin") || asset.name.toLowerCase().endsWith(".zip")) {
				assets.push(asset);
			}
		}
		let selected;
		if (assets.length == 1) selected = assets[0];
		else {
			try {
				selected = await chooseAsset(assets);
			} catch {
				return;
			}
		}

		await installPlugin(plugin.name, selected.browser_download_url, null, id);
	}

	async function installPluginFile() {
		const path = await open({ multiple: false, directory: false });
		if (!path) return;
		await installPlugin(path.split(/[\/\\]/).at(-1) ?? path, null, path, null);
	}

	async function removePlugin(plugin: any) {
		if (
			!(await ask($t("plugin_manager.remove.prompt", { name: plugin.name }), {
				title: $t("plugin_manager.remove.title", { name: plugin.name }),
				okLabel: $t("dialog.yes"),
				cancelLabel: $t("dialog.no"),
			}))
		)
			return;
		try {
			await invoke("remove_plugin", { id: plugin.id });
			message($t("plugin_manager.remove.success", { name: plugin.name }), {
				title: $t("plugin_manager.remove.success.title", { name: plugin.name }),
				buttons: { ok: $t("dialog.ok") },
			});
			$actionList?.reload();
			$deviceSelector?.reloadProfiles();
			installed = await invoke("list_plugins");
		} catch (error: any) {
			message(error, { title: $t("plugin_manager.remove.error", { name: plugin.name }), buttons: { ok: $t("dialog.ok") } });
		}
	}

	async function isUpdateAvailable(plugin: any): Promise<string | false> {
		const id = plugin.id.endsWith(".sdPlugin") ? plugin.id.slice(0, -9) : plugin.id;
		const cataloguePlugin = plugins[id];
		if (!cataloguePlugin) return false;
		const normalize = (v: string) => String(v).replace(/^v/, "").replace(/^(\d+\.\d+\.\d+)\.\d+$/, "$1");
		// Orvo's catalogue already knows the newest version: no GitHub request needed.
		if (cataloguePlugin.version) return normalize(cataloguePlugin.version) != normalize(plugin.version) ? cataloguePlugin.version.replace(/^v/, "") : false;
		if (cataloguePlugin.download_url) return false;

		try {
			const endpoint = new URL(cataloguePlugin.repository);
			endpoint.hostname = "api." + endpoint.hostname;
			endpoint.pathname = "/repos" + endpoint.pathname + "/releases/latest";

			const res = await fetch(endpoint);
			if (!res.ok) return false;
			const release = await res.json();

			const normalizeVersion = (v: string) => v.replace(/^v/, "").replace(/^(\d+\.\d+\.\d+)\.\d+$/, "$1");
			if (normalizeVersion(release.tag_name) != normalizeVersion(plugin.version)) {
				return release.tag_name.replace(/^v/, "");
			} else {
				return false;
			}
		} catch (error) {
			console.warn("Failed to check for plugin update:", error);
			return false;
		}
	}

	let installed: any[] = [];
	(async () => (installed = await invoke("list_plugins")))();

	let plugins: { [id: string]: GitHubPlugin } = {};

	// Orvo: the plugin store reads Orvo's own daily catalogue (scripts/catalogue/build.py), which merges
	// OpenDeck's catalogue, the community plugin list, GitHub, BarRaider and the old Elgato store
	// archive, and records which systems each plugin really supports.
	const CATALOGUE_URL = "https://raw.githubusercontent.com/Som3Rand0m3Dud3/Orvo-Deck/catalogue/plugins.json";
	type Platform = "windows" | "mac" | "linux";
	type StoreEntry = {
		id: string;
		name: string;
		author?: string;
		description?: string;
		source: "open-source" | "archive";
		repository?: string;
		download_url?: string;
		version?: string;
		updated?: string;
		platforms?: Platform[];
		platforms_known?: boolean;
		icon?: string;
		language?: string;
		flags?: { ai_assisted?: boolean };
	};
	let catalogue: StoreEntry[] | null = null;
	let catalogueFailed = false;
	(async () => {
		try {
			const data = await (await fetch(CATALOGUE_URL)).json();
			catalogue = data.plugins as StoreEntry[];
		} catch {
			// Fall back to OpenDeck's catalogue so the store is never empty.
			try {
				const oa: { [id: string]: GitHubPlugin & { description?: string } } = await (await fetch("https://openactionapi.github.io/plugins/catalogue.json")).json();
				catalogue = Object.entries(oa).map(([id, p]) => ({
					id,
					name: p.name,
					author: p.author,
					description: p.description,
					source: "open-source",
					repository: p.repository,
					download_url: p.download_url,
					icon: `https://openactionapi.github.io/plugins/icons/${id}.png`,
					flags: p.flags,
				}));
			} catch {
				catalogueFailed = true;
				catalogue = [];
			}
		}
		plugins = Object.fromEntries(
			catalogue.filter((e) => e.source != "archive").map((e) => [e.id, { name: e.name, author: e.author ?? "", repository: e.repository ?? "", download_url: e.download_url, flags: e.flags, icon: e.icon, version: e.version }]),
		);
	})();

	// Opening a plugin file (or Orvo started that way) installs it.
	listen("elgato_install", ({ payload }: { payload: { name: string; url: string | null; file: string | null } }) => {
		showPopup = true;
		installPlugin(payload.name, payload.url, payload.file, null, false);
	});
	(async () => {
		const pending: { name: string; url: string | null; file: string | null }[] = await invoke("orvo_take_pending_installs");
		for (const p of pending ?? []) {
			showPopup = true;
			await installPlugin(p.name, p.url, p.file, null, false);
		}
	})();
	listen("elgato_notice", ({ payload }: { payload: unknown }) => {
		message(typeof payload == "string" ? payload : "Orvo couldn't install this plugin.", { title: "Plugins", buttons: { ok: $t("dialog.ok") } });
	});

	// Tabs, and which system's plugins to show (this PC's by default).
	type StoreTab = "all" | "orvo" | "open" | "archive";
	let storeTab: StoreTab = "all";
	const STORE_TABS: [StoreTab, string][] = [
		["all", "All"],
		["orvo", "Orvo"],
		["open", "Open-source"],
		["archive", "Elgato archive"],
	];
	type OsFilter = Platform | "any";
	const thisOs: Platform = /Win/i.test(navigator.userAgent) ? "windows" : /Mac/i.test(navigator.userAgent) ? "mac" : "linux";
	let osFilter: OsFilter = thisOs;
	try {
		const saved = localStorage.getItem("orvo-store-os");
		if (saved == "windows" || saved == "mac" || saved == "linux" || saved == "any") osFilter = saved;
	} catch {}
	$: try {
		localStorage.setItem("orvo-store-os", osFilter);
	} catch {}
	const OS_CHOICES: OsFilter[] = ["windows", "mac", "linux", "any"];
	const OS_LABEL: { [k in OsFilter]: string } = { windows: "Windows", mac: "macOS", linux: "Linux", any: "Any system" };

	// Language: English by default. Plugins whose language is unknown count as English.
	const LANG_NAMES: { [code: string]: string } = {
		en: "English", es: "Spanish", de: "German", fr: "French", pt: "Portuguese", it: "Italian", nl: "Dutch", pl: "Polish",
		ru: "Russian", uk: "Ukrainian", cs: "Czech", sv: "Swedish", tr: "Turkish", ar: "Arabic", zh: "Chinese", ja: "Japanese", ko: "Korean",
	};
	const langOf = (e: StoreEntry) => e.language ?? "en";
	let langFilter = "en";
	try {
		langFilter = localStorage.getItem("orvo-store-lang") ?? "en";
	} catch {}
	$: try {
		localStorage.setItem("orvo-store-lang", langFilter);
	} catch {}
	$: storeLangs = [...new Set((catalogue ?? []).map(langOf))].filter((l) => l != "en").sort((a, b) => (LANG_NAMES[a] ?? a).localeCompare(LANG_NAMES[b] ?? b));
	const speaks = (e: StoreEntry, lang: string) => lang == "any" || langOf(e) == lang;

	const fits = (e: StoreEntry, os: OsFilter) => os == "any" || !e.platforms_known || !e.platforms || e.platforms.includes(os);
	const matches = (e: StoreEntry, q: string) =>
		!q || e.name?.toLowerCase().includes(q) || e.author?.toLowerCase().includes(q) || e.description?.toLowerCase().includes(q) || e.id.toLowerCase().includes(q);
	$: lowerQuery = query.toLowerCase().trim();
	$: shownEntries = (catalogue ?? []).filter(
		(e) => (storeTab == "all" || (storeTab == "open" ? e.source == "open-source" : storeTab == "archive" ? e.source == "archive" : false)) && fits(e, osFilter) && speaks(e, langFilter) && matches(e, lowerQuery),
	);
	$: countFor = (tab: StoreTab) =>
		tab == "orvo"
			? installed.filter((p) => p.builtin).length
			: (catalogue ?? []).filter((e) => (tab == "all" || (tab == "open" ? e.source == "open-source" : e.source == "archive")) && fits(e, osFilter) && speaks(e, langFilter)).length;

	function openEntry(e: StoreEntry) {
		if (e.source == "archive") installPlugin(e.name, e.download_url ?? null, null, e.id);
		else openDetailsView = e.id;
	}

	let availableUpdates: { [id: string]: string | false } = {};
	let checkedPlugins = new Set<string>();
	$: if (showPopup) {
		for (const plugin of installed) {
			if (!checkedPlugins.has(plugin.id)) {
				checkedPlugins.add(plugin.id);
				isUpdateAvailable(plugin).then((version) => (availableUpdates = { ...availableUpdates, [plugin.id]: version }));
			}
		}
	}

	let pluginVersions: { [id: string]: string } = {};
	$: for (const plugin of installed) {
		if (pluginVersions[plugin.id] != plugin.version) {
			checkedPlugins.delete(plugin.id);
			delete availableUpdates[plugin.id];
			availableUpdates = availableUpdates;
			pluginVersions[plugin.id] = plugin.version;
		}
	}

	let query: string = "";

	onOpenUrl((urls: string[]) => {
		if (!urls[0].includes("installPlugin/")) return;
		let id = urls[0].split("installPlugin/")[1];
		if (!plugins[id]) return;
		installPluginGitHub(id, plugins[id]);
	});
</script>

<button
	class="px-3 py-1 text-sm text-neutral-300 bg-neutral-700 hover:bg-neutral-600 transition-colors border border-neutral-600 rounded-lg"
	on:click={() => (showPopup = true)}
>
	{$t("plugin_manager.button")}
</button>

<svelte:window
	on:keydown={(event) => {
		if (event.key == "Escape") {
			if (choices) cancelChoice();
			else if (openDetailsView) openDetailsView = null;
			else showPopup = false;
		}
	}}
/>

<Popup show={showPopup} label={$t("plugin_manager.title")}>
	<svelte:fragment slot="header">
		<div class="flex flex-row items-center gap-2">
			<h2 class="m-2 grow font-semibold text-xl text-neutral-300">{$t("plugin_manager.title")}</h2>
			<button class="mr-2 my-1 text-xl text-neutral-300" on:click={() => (showPopup = false)} aria-label={$t("settings.close")}>✕</button>
		</div>
	</svelte:fragment>

	<h2 class="mx-2 mt-4 mb-2 text-lg text-neutral-400">{$t("plugin_manager.installed")}</h2>
	<div class="grid grid-cols-2 lg:grid-cols-3 xl:grid-cols-4">
		<!-- prettier-ignore -->
		{#each installed.sort((a, b) =>
			(a.builtin && !b.builtin) ? -1 :
			(b.builtin && !a.builtin) ? 1 :
			(a.has_settings_interface && !b.has_settings_interface) ? -1 :
			(b.has_settings_interface && !a.has_settings_interface) ? 1 :
			a.id.localeCompare(b.id)
		) as plugin}
			<ListedPlugin
				icon={getWebserverUrl(plugin.icon)}
				name={($localisations && $localisations[plugin.id] && $localisations[plugin.id].Name) ? $localisations[plugin.id].Name : plugin.name}
				subtitle={plugin.version}
				disconnected={!plugin.registered}
				action={() => {
					if ($settings?.developer) invoke("reload_plugin", { id: plugin.id });
					else removePlugin(plugin);
				}}
				actionLabel={$settings?.developer ? $t("plugin_manager.reload") : $t("plugin_manager.remove")}}
				secondaryAction={!plugin.registered ? () => invoke("open_log_directory") : plugin.has_settings_interface ? () => invoke("show_settings_interface", { plugin: plugin.id }) : undefined}
				secondaryActionLabel={!plugin.registered ? $t("plugin_manager.view_logs") : $t("plugin_manager.plugin_settings")}}
			>
				<svelte:fragment slot="subtitle">
					{plugin.version}
					{#if availableUpdates[plugin.id]}
						(<span class="text-yellow-400">
							{$t("plugin_manager.available")}
							<button
								class="font-semibold underline"
								on:click={() => openDetailsView = plugin.id.endsWith(".sdPlugin") ? plugin.id.slice(0, -9) : plugin.id}
							>
								{availableUpdates[plugin.id]}
							</button></span>)
					{/if}
				</svelte:fragment>

				<svelte:fragment slot="secondary">
					{#if !plugin.registered}
						<WarningCircle size="24" class="text-yellow-500" />
					{:else if plugin.has_settings_interface}
						<Gear size="24" class="text-green-600" />
					{/if}
				</svelte:fragment>

				{#if $settings?.developer}
					<ArrowClockwise size="24" class="mt-2 text-neutral-400" />
				{:else if !plugin.builtin}
					<Trash size="24" class="mt-2 text-neutral-400" />
				{/if}
			</ListedPlugin>
		{/each}
	</div>


	<div class="flex flex-row items-center gap-2 mx-2 mt-6 mb-2">
		<h2 class="grow text-lg text-neutral-400">{$t("plugin_manager.store")}</h2>
		<button
			class="flex flex-row items-center mt-2 px-1 py-0.5 text-sm text-neutral-300 bg-neutral-700 hover:bg-neutral-600 transition-colors border border-neutral-600 rounded-lg"
			on:click={installPluginFile}
		>
			<FileArrowUp />
			<span class="ml-1">{$t("plugin_manager.install_from_file")}</span>
		</button>
	</div>

	<div class="flex flex-row flex-wrap items-center gap-3 mx-2 mt-2">
		<div role="tablist" aria-label="Plugin types" class="flex gap-1 p-1 rounded-xl bg-neutral-700 w-fit">
			{#each STORE_TABS as [id, label]}
				<button
					role="tab"
					aria-selected={storeTab == id}
					class="flex items-center gap-1.5 px-4 h-9 rounded-lg text-sm transition-colors {storeTab == id ? 'bg-neutral-900 text-neutral-100' : 'text-neutral-300 hover:text-neutral-100'}"
					on:click={() => (storeTab = id)}
					>{label}{#if catalogue}<span class="text-xs text-neutral-400">{countFor(id)}</span>{/if}</button
				>
			{/each}
		</div>
		<div class="grow"></div>
		<div role="group" aria-label="Show plugins for" class="flex items-center gap-1">
			<span class="mr-1 text-sm text-neutral-400">Works on</span>
			{#each OS_CHOICES as os}
				<button
					class="h-8 px-3 rounded-full border text-sm transition-colors {osFilter == os ? 'text-neutral-100 bg-neutral-900' : 'text-neutral-300 border-neutral-600 hover:text-neutral-100'}"
					style={osFilter == os ? "border-color: var(--orvo-accent)" : ""}
					aria-pressed={osFilter == os}
					on:click={() => (osFilter = os)}>{OS_LABEL[os]}</button
				>
			{/each}
		</div>
		<label class="flex items-center gap-2 text-sm text-neutral-400">
			Language
			<div class="select-wrapper">
				<select bind:value={langFilter} class="h-8 pl-3! pr-9! rounded-full! bg-neutral-700 border border-neutral-600 text-neutral-200" aria-label="Plugin language">
					<option value="en">English</option>
					<option value="any">All languages</option>
					{#each storeLangs as code}<option value={code}>{LANG_NAMES[code] ?? code.toUpperCase()}</option>{/each}
				</select>
			</div>
		</label>
	</div>

	{#if storeTab != "orvo"}
		<div class="flex flex-row items-center m-2 bg-neutral-700 border border-neutral-600 rounded-lg">
			<MagnifyingGlass size="14" class="ml-3 mr-0.5 text-neutral-300" />
			<input
				bind:value={query}
				class="w-full p-2 text-neutral-300"
				placeholder={$t("plugin_manager.search")}
				aria-label={$t("plugin_manager.search")}
				type="search"
				spellcheck="false"
			/>
		</div>

		<div class="flex flex-row items-center mx-2 my-3 p-3 space-x-2 bg-yellow-900/20 border-l-4 border-yellow-500 rounded">
			<WarningCircle size="20" class="mt-0.5 text-yellow-500" />
			<div class="text-sm text-yellow-200">
				{$t("plugin_manager.warning", { PRODUCT_NAME })}
				{#if storeTab == "archive"}Archived plugins are old copies from Elgato's former store and may no longer work.{/if}
			</div>
		</div>
	{/if}

	{#if storeTab == "orvo"}
		<p class="mx-2 mt-4 mb-2 text-sm text-neutral-400">Plugins made by Orvo. More are on the way.</p>
		<div class="grid grid-cols-2 lg:grid-cols-3 xl:grid-cols-4">
			{#each installed.filter((p) => p.builtin) as plugin (plugin.id)}
				<ListedPlugin icon={getWebserverUrl(plugin.icon)} name={plugin.name} subtitle={`Installed · ${plugin.version}`} action={() => {}} actionLabel="Installed">
					<span class="text-xs text-emerald-400">Installed</span>
				</ListedPlugin>
			{/each}
		</div>
	{:else if !catalogue}
		<h2 class="mx-2 mt-6 mb-2 text-md text-neutral-400">{$t("plugin_manager.loading.open_source")}</h2>
	{:else}
		{#if catalogueFailed}
			<p class="mx-2 mt-4 text-sm text-neutral-400">The plugin list couldn't be loaded. Check your internet connection and open Plugins again.</p>
		{:else if shownEntries.length == 0}
			<p class="mx-2 mt-4 text-sm text-neutral-400">No plugins found{osFilter != "any" ? ` for ${OS_LABEL[osFilter]}` : ""}{langFilter != "any" ? ` in ${LANG_NAMES[langFilter] ?? langFilter}` : ""}.</p>
		{/if}
		<div class="grid grid-cols-2 lg:grid-cols-3 xl:grid-cols-4">
			{#each shownEntries as entry (entry.source + entry.id)}
				<ListedPlugin
					icon={entry.icon ?? ""}
					name={entry.name}
					subtitle={entry.author ?? ""}
					action={() => openEntry(entry)}
					actionLabel={entry.source == "archive" ? $t("plugin_details.install") : $t("plugin_manager.view_details")}
				>
					<svelte:fragment slot="subtitle">
						<span class="flex flex-wrap items-center gap-1">
							<span>{entry.author ?? ""}</span>
							{#if entry.flags?.ai_assisted}<Robot size="1em" class="inline align-text-bottom text-yellow-500" />{/if}
							{#if entry.platforms_known && entry.platforms}
								{#each entry.platforms as os}
									<span class="px-1.5 rounded text-[10px] leading-4 bg-neutral-700 text-neutral-300">{os == "windows" ? "Win" : os == "mac" ? "Mac" : "Linux"}</span>
								{/each}
							{:else if osFilter != "any"}
								<span class="px-1.5 rounded text-[10px] leading-4 bg-neutral-700 text-yellow-300" title="This plugin doesn't say which systems it supports">Untested on {OS_LABEL[osFilter]}</span>
							{/if}
							{#if entry.source == "archive"}<span class="px-1.5 rounded text-[10px] leading-4 bg-neutral-700 text-neutral-400">Old version</span>{/if}
							{#if langOf(entry) != "en"}<span class="px-1.5 rounded text-[10px] leading-4 bg-neutral-700 text-sky-300" title="This plugin is in {LANG_NAMES[langOf(entry)] ?? langOf(entry)}">{LANG_NAMES[langOf(entry)] ?? langOf(entry).toUpperCase()}</span>{/if}
						</span>
					</svelte:fragment>
					{#if entry.source == "archive"}
						<CloudArrowDown size="24" class="text-neutral-400" />
					{:else}
						<ArrowSquareOut size="24" class="text-neutral-400" />
					{/if}
				</ListedPlugin>
			{/each}
		</div>
	{/if}
</Popup>

{#if openDetailsView}
	<PluginDetails
		id={openDetailsView}
		details={plugins[openDetailsView]}
		install={() => {
			// @ts-expect-error
			installPluginGitHub(openDetailsView, plugins[openDetailsView]);
		}}
		close={() => (openDetailsView = null)}
	/>
{/if}

{#if choices}
	<div
		class="fixed left-1/2 top-1/2 -translate-x-1/2 -translate-y-1/2 mt-2 p-2 w-96 text-xs text-neutral-300 bg-neutral-700 border border-neutral-600 rounded-lg z-40"
	>
		<h3 class="mb-2 font-semibold text-lg text-center">{$t("plugin_manager.choose_asset")}</h3>
		<div class="select-wrapper">
			<select class="w-full bg-neutral-800!" bind:value={choice} aria-label={$t("plugin_manager.choose_asset.label")}>
				{#each choices as choice, i}
					<option value={i}>{choice.name}</option>
				{/each}
			</select>
		</div>
		<button
			class="mt-2 p-1 w-full text-sm text-neutral-300 bg-neutral-800 hover:bg-neutral-900 transition-colors border border-neutral-600 rounded-lg"
			on:click={finishChoice}
		>
			{$t("plugin_details.install")}
		</button>
	</div>
{/if}

{#if installing}
	<div
		role="status"
		aria-live="polite"
		transition:fly={{ y: 24, duration: 180 }}
		class="fixed left-1/2 bottom-6 -translate-x-1/2 w-[26rem] max-w-[calc(100%-2rem)] p-4 bg-neutral-900 border border-neutral-600 rounded-xl shadow-2xl z-[70]"
	>
		<p class="font-semibold text-neutral-100 truncate">Installing {installing.name}</p>
		<p class="mt-0.5 text-sm text-neutral-400">{installLabel}</p>
		<div class="relative mt-3 h-2 overflow-hidden rounded-full bg-neutral-700">
			{#if installPercent != null}
				<div class="h-full rounded-full transition-[width] duration-150" style="width: {installPercent}%; background: var(--orvo-accent)"></div>
			{:else}
				<div class="orvo-indeterminate absolute inset-y-0 w-1/3 rounded-full" style="background: var(--orvo-accent)"></div>
			{/if}
		</div>
	</div>
{/if}

<style>
	.orvo-indeterminate {
		animation: orvo-indeterminate 1.1s ease-in-out infinite;
	}
	@keyframes orvo-indeterminate {
		from {
			left: -33%;
		}
		to {
			left: 100%;
		}
	}
	@media (prefers-reduced-motion: reduce) {
		.orvo-indeterminate {
			animation-duration: 3s;
		}
	}
</style>
