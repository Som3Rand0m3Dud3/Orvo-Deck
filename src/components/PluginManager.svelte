<script lang="ts">
	import ArrowClockwise from "phosphor-svelte/lib/ArrowClockwise";
	import ArrowSquareOut from "phosphor-svelte/lib/ArrowSquareOut";
	import CloudArrowDown from "phosphor-svelte/lib/CloudArrowDown";
	import FileArrowUp from "phosphor-svelte/lib/FileArrowUp";
	import Gear from "phosphor-svelte/lib/Gear";
	import MagnifyingGlass from "phosphor-svelte/lib/MagnifyingGlass";
	import Robot from "phosphor-svelte/lib/Robot";
	import SignOut from "phosphor-svelte/lib/SignOut";
	import Storefront from "phosphor-svelte/lib/Storefront";
	import UserCircle from "phosphor-svelte/lib/UserCircle";
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
	import { ask, message, openDialog } from "$lib/dialog";
	import { onDestroy, tick } from "svelte";
	import { open } from "@tauri-apps/plugin-dialog";

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
		try {
			await invoke("install_plugin", { url, file, fallback_id });
			message($t("plugin_manager.install.success", { name }), {
				title: $t("plugin_manager.install.success.title", { name }),
				buttons: { ok: $t("dialog.ok") },
			});
			$actionList?.reload();
			installed = await invoke("list_plugins");
		} catch (error: any) {
			message(error, { title: $t("plugin_manager.install.error", { name }), buttons: { ok: $t("dialog.ok") } });
		}
	}

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
		flags?: {
			ai_assisted?: boolean;
		};
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
		try {
			res = await (await fetch(endpoint)).json();
		} catch (error: any) {
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
		if (!cataloguePlugin || cataloguePlugin.download_url) return false;

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

	let plugins: { [id: string]: GitHubPlugin };
	(async () => (plugins = await (await fetch("https://openactionapi.github.io/plugins/catalogue.json")).json()))();

	// Orvo: Elgato Marketplace. It opens in its own window; Orvo installs what you get there.
	type ElgatoAccount = { signed_in: boolean; name: string; email: string };
	let elgato: ElgatoAccount = { signed_in: false, name: "", email: "" };
	try {
		elgato = { ...elgato, ...JSON.parse(localStorage.getItem("orvo-elgato-account") ?? "{}") };
	} catch {}
	let accountMenu = false;
	$: elgatoLabel = elgato.name || elgato.email || "Elgato account";
	listen("elgato_account", ({ payload }: { payload: ElgatoAccount }) => {
		elgato = payload;
		try {
			localStorage.setItem("orvo-elgato-account", JSON.stringify(payload));
		} catch {}
	});
	listen("elgato_install", ({ payload }: { payload: { name: string; url: string | null; file: string | null } }) => {
		showPopup = true;
		storeTab = "elgato";
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
		const link = typeof payload == "string" && payload ? `\n\nLink received: ${payload}` : "";
		message(`Orvo couldn't install this plugin by itself. Download the plugin file from its Marketplace page, then open it or use "Install from file".${link}`, {
			title: "Elgato Marketplace",
			buttons: { ok: $t("dialog.ok") },
		});
	});
	function openMarketplace() {
		accountMenu = false;
		showPopup = true;
		storeTab = "elgato";
	}

	// The Marketplace is a separate web view laid over the Elgato tab's area. It is hidden whenever
	// something of Orvo's must show on top (messages, menus, details), since it would cover them.
	let mpEl: HTMLDivElement | null = null;
	let mpHeight = 520;
	let mpShown = false;
	let mpRect = "";
	let mpFrame = 0;
	$: wantMarketplace = showPopup && storeTab == "elgato" && !$openDialog && !accountMenu && !openDetailsView && !choices;
	function placeMarketplace() {
		cancelAnimationFrame(mpFrame);
		mpFrame = requestAnimationFrame(async () => {
			if (!wantMarketplace || !mpEl) {
				if (mpShown) {
					mpShown = false;
					mpRect = "";
					invoke("orvo_marketplace_hide").catch(() => {});
				}
				return;
			}
			const height = Math.max(240, Math.floor(window.innerHeight - mpEl.getBoundingClientRect().top - 28));
			if (Math.abs(height - mpHeight) > 1) {
				mpHeight = height;
				await tick();
			}
			const r = mpEl.getBoundingClientRect();
			const key = [r.left, r.top, r.width, r.height].map(Math.round).join(",");
			if (mpShown && key == mpRect) return;
			mpShown = true;
			mpRect = key;
			invoke("orvo_marketplace_show", { x: r.left, y: r.top, width: r.width, height: r.height }).catch((error) => console.error(error));
		});
	}
	$: wantMarketplace, mpEl, placeMarketplace();
	function watchSize(node: HTMLElement) {
		const observer = new ResizeObserver(() => placeMarketplace());
		observer.observe(node);
		return { destroy: () => observer.disconnect() };
	}
	onDestroy(() => invoke("orvo_marketplace_hide").catch(() => {}));
	const MARKETPLACE = "https://marketplace.elgato.com/stream-deck/plugins";
	async function openInBrowser() {
		accountMenu = false;
		await invoke("open_url", { url: MARKETPLACE });
	}
	async function signOutElgato() {
		accountMenu = false;
		try {
			await invoke("orvo_marketplace_sign_out");
		} catch (error: any) {
			message(String(error), { title: "Elgato Marketplace", buttons: { ok: $t("dialog.ok") } });
		}
	}

	// Orvo: the store has tabs. "All" shows (and searches) every kind; Elgato is coming later.
	type StoreTab = "all" | "open" | "elgato";
	let storeTab: StoreTab = "all";
	const STORE_TABS: [StoreTab, string][] = [
		["all", "All"],
		["open", "Open-source"],
		["elgato", "Elgato"],
	];

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
	on:resize={placeMarketplace}
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
			<div class="relative">
				{#if elgato.signed_in}
					<button
						class="flex items-center gap-2 h-9 px-3 rounded-full bg-neutral-700 border border-neutral-600 text-sm text-neutral-200 hover:bg-neutral-600 transition-colors"
						aria-haspopup="menu"
						aria-expanded={accountMenu}
						title="Signed in to the Elgato Marketplace"
						on:click={() => (accountMenu = !accountMenu)}
					>
						<UserCircle size="18" />
						<span class="max-w-48 truncate">{elgatoLabel}</span>
					</button>
					{#if accountMenu}
						<div role="menu" class="absolute right-0 top-11 z-40 w-56 p-1 rounded-lg bg-neutral-700 border border-neutral-600 shadow-lg">
							{#if elgato.email && elgato.name}<p class="px-3 py-2 text-xs text-neutral-400 truncate">{elgato.email}</p>{/if}
							<button role="menuitem" class="flex items-center gap-2 w-full px-3 py-2 rounded-md text-sm text-left text-neutral-200 hover:bg-neutral-600" on:click={openMarketplace}>
								<Storefront size="16" />Go to the Marketplace
							</button>
							<button role="menuitem" class="flex items-center gap-2 w-full px-3 py-2 rounded-md text-sm text-left text-neutral-200 hover:bg-neutral-600" on:click={openInBrowser}>
								<ArrowSquareOut size="16" />Open in my browser
							</button>
							<button role="menuitem" class="flex items-center gap-2 w-full px-3 py-2 rounded-md text-sm text-left text-neutral-200 hover:bg-neutral-600" on:click={signOutElgato}>
								<SignOut size="16" />Sign out
							</button>
						</div>
					{/if}
				{:else}
					<button
						class="flex items-center gap-2 h-9 px-3 rounded-full bg-neutral-700 border border-neutral-600 text-sm text-neutral-200 hover:bg-neutral-600 transition-colors"
						title="Sign in inside the Elgato tab"
						on:click={openMarketplace}
					>
						<UserCircle size="18" />Not signed in
					</button>
				{/if}
			</div>
			<button class="mr-2 my-1 text-xl text-neutral-300" on:click={() => (showPopup = false)} aria-label={$t("settings.close")}>✕</button>
		</div>
	</svelte:fragment>

	{#if storeTab != "elgato"}
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

	{/if}

	<div class="flex flex-row items-center gap-2 mx-2 mt-6 mb-2" class:mt-2={storeTab == "elgato"}>
		<h2 class="grow text-lg text-neutral-400">{$t("plugin_manager.store")}</h2>
		<button
			class="flex flex-row items-center mt-2 px-1 py-0.5 text-sm text-neutral-300 bg-neutral-700 hover:bg-neutral-600 transition-colors border border-neutral-600 rounded-lg"
			title="Google sign-in works there. Press Open in Stream Deck, or download the plugin and open the file, and Orvo installs it."
			on:click={openInBrowser}
		>
			<ArrowSquareOut />
			<span class="ml-1">Open in my browser</span>
		</button>
		<button
			class="flex flex-row items-center mt-2 px-1 py-0.5 text-sm text-neutral-300 bg-neutral-700 hover:bg-neutral-600 transition-colors border border-neutral-600 rounded-lg"
			on:click={installPluginFile}
		>
			<FileArrowUp />
			<span class="ml-1">{$t("plugin_manager.install_from_file")}</span>
		</button>
	</div>

	<div role="tablist" aria-label="Plugin types" class="flex gap-1 mx-2 mt-2 p-1 rounded-xl bg-neutral-700 w-fit">
		{#each STORE_TABS as [id, label]}
			<button
				role="tab"
				aria-selected={storeTab == id}
				class="px-4 h-9 rounded-lg text-sm transition-colors {storeTab == id ? 'bg-neutral-900 text-neutral-100' : 'text-neutral-300 hover:text-neutral-100'}"
				on:click={() => (storeTab = id)}>{label}</button
			>
		{/each}
	</div>

	{#if storeTab != "elgato"}
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

	<div class="flex flex-row items-center mx-2 my-4 p-3 space-x-2 bg-yellow-900/20 border-l-4 border-yellow-500 rounded">
		<WarningCircle size="20" class="mt-0.5 text-yellow-500" />
		<div class="text-sm text-yellow-200">
			{$t("plugin_manager.warning", { PRODUCT_NAME })}
		</div>
	</div>

	{/if}

	{#if storeTab != "elgato"}
		{#if !plugins}
			<h2 class="mx-2 mt-6 mb-2 text-md text-neutral-400">{$t("plugin_manager.loading.open_source")}</h2>
		{:else}
			<div class="flex flex-row items-center ml-2 mt-6 mb-2 space-x-2">
				<h2 class="font-semibold text-md text-neutral-400">{$t("plugin_manager.open_source")}</h2>
				<Tooltip>{$t("plugin_manager.open_source.tooltip")}</Tooltip>
			</div>
			<div class="grid grid-cols-2 lg:grid-cols-3 xl:grid-cols-4">
				{#each Object.entries(plugins) as [id, plugin]}
					<ListedPlugin
						icon="https://openactionapi.github.io/plugins/icons/{id}.png"
						name={plugin.name}
						subtitle={plugin.author}
						hidden={!plugin.name.toLowerCase().includes(query.toLowerCase())}
						action={() => (openDetailsView = id)}
						actionLabel={$t("plugin_manager.view_details")}
					>
						<svelte:fragment slot="subtitle">
							<span>
								{plugin.author}
								{#if plugin.flags?.ai_assisted}
									<Robot size="1em" class="inline align-text-bottom text-yellow-500" />
								{/if}
							</span>
						</svelte:fragment>

						<ArrowSquareOut size="24" class="text-neutral-400" />
					</ListedPlugin>
				{/each}
			</div>
		{/if}
	{:else}
		<div class="flex flex-row items-center gap-2 mx-2 mt-3 mb-2">
			<button class="flex items-center justify-center w-9 h-9 rounded-lg bg-neutral-700 border border-neutral-600 text-neutral-200 hover:bg-neutral-600" aria-label="Back" title="Back" on:click={() => invoke("orvo_marketplace_back")}>←</button>
			<button class="flex items-center gap-1.5 h-9 px-3 rounded-lg bg-neutral-700 border border-neutral-600 text-sm text-neutral-200 hover:bg-neutral-600" on:click={() => invoke("orvo_marketplace_home")}>
				<Storefront size="16" />Plugins
			</button>
			<p class="ml-2 text-xs text-neutral-400">
				Sign in inside the Marketplace (email, Discord or Twitch). Press <strong>Open in Stream Deck</strong> or <strong>Download</strong>, and Orvo installs it. Google sign-in: "Open in my browser".
			</p>
		</div>
		<div bind:this={mpEl} use:watchSize class="mx-2 rounded-xl border border-neutral-600 bg-neutral-900 flex items-center justify-center text-sm text-neutral-500" style={`height: ${mpHeight}px`}>
			Loading the Elgato Marketplace…
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
