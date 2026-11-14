<script lang="ts">
	import { appUpdate } from "$lib/orvoUpdate";
	import ClockClockwise from "phosphor-svelte/lib/ClockClockwise";
	import ClockCounterClockwise from "phosphor-svelte/lib/ClockCounterClockwise";
	import Gear from "phosphor-svelte/lib/Gear";
	import Scroll from "phosphor-svelte/lib/Scroll";
	import Popup from "./Popup.svelte";
	import Tooltip from "./Tooltip.svelte";

	import { t } from "$lib/i18n";
	import { settings } from "$lib/settings";
	import { PRODUCT_NAME } from "$lib/singletons";

	import { invoke } from "@tauri-apps/api/core";
	import { listen } from "@tauri-apps/api/event";
	import { ask, message } from "$lib/dialog";

	type Release = { tag: string; version: string; name: string; notes: string; date: string; page: string; installer: string | null; newer: boolean; current: boolean; prerelease: boolean };
	let releases: Release[] | null = null;
	let selectedTag = "";
	let updateState: "idle" | "checking" | "error" | "installing" = "idle";
	$: currentVersion = buildInfo?.match(/v(\d+\.\d+\.\d+[^\s<]*)/)?.[1] ?? "";
	$: newest = releases?.find((r) => r.newer);

	// Page slide speed on the pad (ms, 0 = off)
	const SLIDE_PRESETS: [string, number][] = [["Off", 0], ["Fast", 110], ["Normal", 180], ["Slow", 320]];
	let slideMs = 180;
	try {
		const saved = localStorage.getItem("orvo-slide-ms");
		if (saved != null) slideMs = parseInt(saved) || 0;
	} catch {}
	function setSlide(value: number) {
		slideMs = value;
		try {
			localStorage.setItem("orvo-slide-ms", String(value));
		} catch {}
		invoke("orvo_pad_settings", { slideMs: value }).catch(() => {});
	}

	// Slider: linear (B) or audio-taper (A) pot, and calibration
	let taper = 0;
	try {
		taper = localStorage.getItem("orvo-slider-taper") == "1" ? 1 : 0;
	} catch {}
	function setTaper(value: number) {
		taper = value;
		try {
			localStorage.setItem("orvo-slider-taper", String(value));
		} catch {}
		invoke("orvo_pad_settings", { taper: value }).catch(() => {});
	}
	let flip = false;
	try {
		flip = localStorage.getItem("orvo-slider-flip") == "1";
	} catch {}
	function setFlip(value: boolean) {
		flip = value;
		try {
			localStorage.setItem("orvo-slider-flip", value ? "1" : "0");
		} catch {}
		invoke("orvo_pad_settings", { flip: value }).catch(() => {});
	}
	function recalibrate() {
		invoke("orvo_pad_settings", { recalibrate: true }).catch(() => {});
		message("Look at the pad's screen. Slide to each point it asks for and press key 1:\n\n1. Bottom\n2. One quarter up\n3. Middle\n4. Three quarters up\n5. Top", { title: "Slider calibration (test)", kind: "info" });
	}

	let testVersions = true;
	try {
		const saved = localStorage.getItem("orvo-test-versions");
		if (saved != null) testVersions = saved == "1";
	} catch {}
	function setTestVersions(value: boolean) {
		testVersions = value;
		try {
			localStorage.setItem("orvo-test-versions", value ? "1" : "0");
		} catch {}
		if (releases) checkUpdates();
	}

	async function checkUpdates() {
		updateState = "checking";
		try {
			releases = await invoke<Release[]>("list_releases", { includePrerelease: testVersions });
			appUpdate.set(releases.find((r) => r.newer) ? { tag: releases.find((r) => r.newer)!.tag } : null);
			selectedTag = (releases.find((r) => r.newer) ?? releases.find((r) => r.current) ?? releases[0])?.tag ?? "";
			updateState = "idle";
		} catch {
			updateState = "error";
		}
	}

	async function installSelected() {
		const release = releases?.find((r) => r.tag == selectedTag);
		if (!release || release.current) return;
		const prompt = release.newer ? $t("settings.updates.confirm", { TAG: release.tag }) : $t("settings.updates.rollback", { TAG: release.tag });
		if (!(await ask(prompt, { title: $t("settings.updates.title"), kind: release.newer ? "info" : "warning" }))) return;
		updateState = "installing";
		try {
			await invoke("install_release", { tag: release.tag });
		} catch (error) {
			updateState = "error";
			await message(String(error), { title: $t("settings.updates.title"), kind: "error" });
		}
		updateState = "idle";
	}

	let showPopup: boolean;
	// opening Settings checks for updates straight away
	let wasOpen = false;
	$: if (showPopup && !wasOpen) {
		wasOpen = true;
		if (updateState != "installing") checkUpdates();
	} else if (!showPopup) wasOpen = false;

	let theme = document.documentElement.dataset.theme == "light" ? "light" : "dark";
	function setTheme(value: "dark" | "light") {
		theme = value;
		document.documentElement.dataset.theme = value;
		try {
			localStorage.setItem("orvo-theme", value);
		} catch {}
	}
	let buildInfo: string;
	(async () => (buildInfo = await invoke("get_build_info")))();

	listen("device_brightness", ({ payload }: { payload: { action: string; value: number } }) => {
		if (!$settings) return;
		let value = $settings.brightness;
		switch (payload.action) {
			case "increase":
				value += payload.value;
				break;
			case "decrease":
				value -= payload.value;
				break;
			default:
				value = payload.value;
				break;
		}
		$settings.brightness = Math.max(0, Math.min(100, value));
	});

	async function backupConfig() {
		await message($t("settings.backup_config.prompt"), { title: $t("settings.backup_config.title"), buttons: { ok: $t("dialog.ok") } });
		if (await invoke("backup_config_directory")) {
			await message($t("settings.backup_config.success.prompt"), { title: $t("settings.backup_config.success.title"), buttons: { ok: $t("dialog.ok") } });
		}
	}

	async function restoreConfig() {
		await message($t("settings.restore_config.prompt"), { title: $t("settings.restore_config.title"), buttons: { ok: $t("dialog.ok") } });
		await invoke("restore_config_directory");
	}
</script>

<button
	class="relative px-3 py-1 text-sm text-neutral-300 bg-neutral-700 hover:bg-neutral-600 transition-colors border border-neutral-600 rounded-lg"
	on:click={() => (showPopup = true)}
	title={$appUpdate ? `Orvo Studio ${$appUpdate.tag} is available` : undefined}
>
	{$t("settings.button")}
	{#if $appUpdate}<span class="absolute -top-1 -right-1 w-2.5 h-2.5 rounded-full bg-emerald-400 ring-2 ring-neutral-900" aria-label="Update available"></span>{/if}
</button>

<svelte:window
	on:keydown={(event) => {
		if (event.key == "Escape") showPopup = false;
	}}
/>

<Popup show={showPopup} label={$t("settings.button")}>
	<svelte:fragment slot="header">
		<button class="mr-2 my-1 float-right text-xl text-neutral-300" on:click={() => (showPopup = false)} aria-label={$t("settings.close")}>✕</button>
		<h2 class="m-2 font-semibold text-xl text-neutral-300">{$t("settings.button")}</h2>
	</svelte:fragment>

	<div class="flex flex-row items-center mx-2 mb-3 space-x-3">
		<span class="text-neutral-400">Appearance:</span>
		<div role="group" aria-label="Appearance" class="flex gap-1 p-1 rounded-xl bg-neutral-700">
			<button class="px-4 h-9 rounded-lg text-sm" class:bg-neutral-900={theme == "dark"} class:text-neutral-100={theme == "dark"} on:click={() => setTheme("dark")}>Dark</button>
			<button class="px-4 h-9 rounded-lg text-sm" class:bg-neutral-900={theme == "light"} class:text-neutral-100={theme == "light"} on:click={() => setTheme("light")}>Light</button>
		</div>
	</div>

	<div class="mx-2 mb-3 p-3 border border-neutral-600 rounded-lg">
		<p class="font-semibold text-neutral-300">Pad</p>
		<div class="flex flex-row flex-wrap items-center gap-2 mt-2">
			<span class="text-neutral-400">Page slide speed:</span>
			{#each SLIDE_PRESETS as [label, ms]}
				<button
					class="px-3 h-9 rounded-lg text-sm border border-neutral-600"
					class:bg-neutral-900={slideMs == ms}
					class:text-neutral-100={slideMs == ms}
					on:click={() => setSlide(ms)}>{label}</button
				>
			{/each}
		</div>
		<div class="flex flex-row items-center gap-3 mt-2">
			<input type="range" min="0" max="500" step="10" value={slideMs} on:change={(e) => setSlide(parseInt(e.currentTarget.value))} aria-label="Page slide speed in milliseconds" />
			<span class="text-sm text-neutral-400 tabular-nums">{slideMs == 0 ? "no slide" : `${slideMs} ms`}</span>
		</div>
		<div class="flex flex-row flex-wrap items-center gap-2 mt-3">
			<span class="text-neutral-400">Slider type:</span>
			<button class="px-3 h-9 rounded-lg text-sm border border-neutral-600" class:bg-neutral-900={taper == 0} class:text-neutral-100={taper == 0} on:click={() => setTaper(0)}>Linear (B)</button>
			<button class="px-3 h-9 rounded-lg text-sm border border-neutral-600" class:bg-neutral-900={taper == 1} class:text-neutral-100={taper == 1} on:click={() => setTaper(1)}>Audio (A)</button>
			<button class="px-3 h-9 rounded-lg text-sm border border-neutral-600" on:click={recalibrate}>Calibrate slider (test)…</button>
			<label class="flex items-center gap-2 ml-2 text-sm text-neutral-400"><input type="checkbox" checked={flip} on:change={(e) => setFlip(e.currentTarget.checked)} /> Flip direction</label>
		</div>
	</div>

	{#if $settings}
		<div class="flex flex-row items-center mx-2 mb-2 space-x-2">
			<label for="settings-language" class="text-neutral-400">{$t("settings.language")}</label>
			<div class="select-wrapper">
				<select bind:value={$settings.language} class="w-auto pr-10!" id="settings-language">
					<option value="de">Deutsch</option>
					<option value="en">English</option>
					<option value="es">Español</option>
					<option value="fr">Français</option>
					<option value="pt_BR">Português (Brasil)</option>
					<option value="sv">Svenska</option>
					<option value="uk">Українська</option>
					<option value="zh_CN">中文</option>
					<option value="ja">日本語</option>
					<option value="ko">한국어</option>
				</select>
			</div>
			<Tooltip>
				{$t("settings.language.tooltip", { PRODUCT_NAME })}
			</Tooltip>
		</div>

		<div class="flex flex-row items-center m-2 space-x-2">
			<label for="settings-brightness" class="text-neutral-400">{$t("settings.brightness")}</label>
			<input type="range" min="0" max="100" bind:value={$settings.brightness} id="settings-brightness" />
		</div>

		<div class="flex flex-row items-center m-2 space-x-2">
			<label for="settings-sleep_timeout_minutes" class="text-neutral-400">{$t("settings.sleep_timeout_minutes")}</label>
			<input
				type="number"
				min="0"
				bind:value={$settings.sleep_timeout_minutes}
				class="w-12 px-1 text-neutral-300 border border-neutral-600 rounded-lg"
				id="settings-sleep_timeout_minutes"
			/>
			<span class="text-neutral-400">{$t("settings.sleep_timeout_minutes.minutes")}</span>
			<Tooltip>{$t("settings.sleep_timeout_minutes.tooltip")}</Tooltip>
		</div>

		<div class="flex flex-row items-center m-2 space-x-2">
			<label for="settings-sleep_when_computer_locked" class="text-neutral-400">{$t("settings.sleep_when_computer_locked")}</label>
			<input type="checkbox" bind:checked={$settings.sleep_when_computer_locked} id="settings-sleep_when_computer_locked" />
			<Tooltip>{$t("settings.sleep_when_computer_locked.tooltip")}</Tooltip>
		</div>

		<div class="flex flex-row items-center m-2 space-x-2">
			<label for="settings-rotation" class="text-neutral-400">{$t("settings.rotation")}</label>
			<input type="range" min="0" max="270" step="90" bind:value={$settings.rotation} id="settings-rotation" />
		</div>

		<div class="flex flex-row items-center m-2 space-x-2">
			<label for="settings-background" class="text-neutral-400">{$t("settings.background")}</label>
			<input type="checkbox" bind:checked={$settings.background} id="settings-background" />
			<Tooltip>{$t("settings.background.tooltip", { PRODUCT_NAME })}</Tooltip>
		</div>

		<div class="flex flex-row items-center m-2 space-x-2">
			<label for="settings-autolaunch" class="text-neutral-400">{$t("settings.autolaunch")}</label>
			<input type="checkbox" bind:checked={$settings.autolaunch} id="settings-autolaunch" />
			<Tooltip>
				{$t("settings.autolaunch.tooltip.1", { PRODUCT_NAME })}
				{#if buildInfo?.split("</summary>")[0]?.includes("linux")}
					<br />
					{$t("settings.autolaunch.tooltip.2", { PRODUCT_NAME })}
				{/if}
			</Tooltip>
		</div>

		<div class="flex flex-row items-center m-2 space-x-2">
			<label for="settings-updatecheck" class="text-neutral-400">{$t("settings.updatecheck")}</label>
			<input type="checkbox" bind:checked={$settings.updatecheck} id="settings-updatecheck" />
		</div>

		<div class="m-2 p-3 border border-neutral-600 rounded-lg">
			<div class="flex flex-row items-center space-x-3">
				<span class="font-semibold text-neutral-300">{$t("settings.updates.title")}</span>
				{#if currentVersion}<span class="text-sm text-neutral-400">{$t("settings.updates.current", { VERSION: currentVersion })}</span>{/if}
				<button
					class="px-2 py-1 text-sm text-neutral-300 bg-neutral-700 hover:bg-neutral-600 transition-colors border border-neutral-600 rounded-lg disabled:opacity-50"
					disabled={updateState == "checking" || updateState == "installing"}
					on:click={checkUpdates}
				>
					{updateState == "checking" ? $t("settings.updates.checking") : $t("settings.updates.check")}
				</button>
			</div>

			<label class="flex items-center gap-2 mt-2 text-sm text-neutral-400">
				<input type="checkbox" checked={testVersions} on:change={(e) => setTestVersions(e.currentTarget.checked)} />
				Show test versions (builds before 1.0)
			</label>

			{#if updateState == "error"}
				<p class="mt-2 text-sm text-red-400">{$t("settings.updates.error")}</p>
			{:else if releases}
				{#if releases.length == 0}
					<p class="mt-2 text-sm text-neutral-400">{$t("settings.updates.none")}</p>
				{:else}
					<p class="mt-2 text-sm {newest ? 'text-green-400' : 'text-neutral-400'}">
						{newest ? $t("settings.updates.available", { TAG: newest.tag }) : $t("settings.updates.latest")}
					</p>
					<div class="flex flex-row items-center mt-2 space-x-2">
						<label for="settings-version" class="text-neutral-400">{$t("settings.updates.choose")}</label>
						<div class="select-wrapper">
							<select bind:value={selectedTag} class="w-auto pr-10!" id="settings-version">
								{#each releases as r, i}
									<option value={r.tag}>
										{r.tag}{r.prerelease ? " · test" : ""}{r.date ? ` (${r.date})` : ""}{i == 0 ? ` · ${$t("settings.updates.newest")}` : ""}{r.current ? ` · ${$t("settings.updates.installed")}` : ""}
									</option>
								{/each}
							</select>
						</div>
						<button
							class="px-3 py-1 text-sm text-white bg-blue-600 hover:bg-blue-500 transition-colors rounded-lg disabled:opacity-50"
							disabled={updateState == "installing" || releases.find((r) => r.tag == selectedTag)?.current}
							on:click={installSelected}
						>
							{updateState == "installing" ? $t("settings.updates.installing") : $t("settings.updates.install")}
						</button>
					</div>
					{#if releases.find((r) => r.tag == selectedTag)?.notes}
						<pre class="mt-2 max-h-32 overflow-auto whitespace-pre-wrap text-xs text-neutral-400">{releases.find((r) => r.tag == selectedTag)?.notes}</pre>
					{/if}
				{/if}
			{/if}
		</div>

		{#if !buildInfo?.split("</summary>")[0]?.includes("windows")}
			<div class="flex flex-row items-center m-2 space-x-2">
				<label for="settings-separatewine" class="text-neutral-400">{$t("settings.separatewine")}</label>
				<input type="checkbox" bind:checked={$settings.separatewine} id="settings-separatewine" />
				<Tooltip>
					{$t("settings.separatewine.tooltip", { PRODUCT_NAME })}
				</Tooltip>
			</div>
		{/if}

		<div class="flex flex-row items-center m-2 space-x-2">
			<label for="settings-developer" class="text-neutral-400">{$t("settings.developer")}</label>
			<input type="checkbox" bind:checked={$settings.developer} id="settings-developer" />
			<Tooltip>
				{$t("settings.developer.tooltip")}
			</Tooltip>
		</div>

		<div class="flex flex-row items-center m-2 space-x-2">
			<label for="settings-disableelgato" class="text-neutral-400">{$t("settings.disableelgato")}</label>
			<input type="checkbox" bind:checked={$settings.disableelgato} id="settings-disableelgato" />
			<Tooltip>{$t("settings.disableelgato.tooltip")}</Tooltip>
		</div>
	{/if}

	<div class="ml-2">
		<div class="flex flex-row my-3 space-x-2">
			<button
				class="flex flex-row items-center px-2 py-1 text-sm text-neutral-300 bg-neutral-700 hover:bg-neutral-600 transition-colors border border-neutral-600 rounded-lg"
				on:click={() => backupConfig()}
			>
				<ClockCounterClockwise class="mr-1" />
				{$t("settings.backup_config.button")}
			</button>
			<button
				class="flex flex-row items-center px-2 py-1 text-sm text-neutral-300 bg-neutral-700 hover:bg-neutral-600 transition-colors border border-neutral-600 rounded-lg"
				on:click={() => restoreConfig()}
			>
				<ClockClockwise class="mr-1" />
				{$t("settings.restore_config.button")}
			</button>
			<button
				class="flex flex-row items-center px-2 py-1 text-sm text-neutral-300 bg-neutral-700 hover:bg-neutral-600 transition-colors border border-neutral-600 rounded-lg"
				on:click={() => invoke("open_config_directory")}
			>
				<Gear class="mr-1" />
				{$t("settings.open_config")}
			</button>
			<button
				class="flex flex-row items-center px-2 py-1 text-sm text-neutral-300 bg-neutral-700 hover:bg-neutral-600 transition-colors border border-neutral-600 rounded-lg"
				on:click={() => invoke("open_log_directory")}
			>
				<Scroll class="mr-1" />
				{$t("settings.open_logs")}
			</button>
		</div>

		<span class="text-xs text-neutral-400">
			{@html buildInfo}
		</span>
	</div>

	<svelte:fragment slot="footer">
		<div class="flex flex-row items-center mt-4 text-sm text-neutral-400">
			<span>
				{$t("settings.footer.1")}
				<button on:click={() => invoke("open_url", { url: "https://github.com/nekename/OpenDeck" })} class="underline">OpenDeck</button>
				{$t("settings.footer.3")}
				<button on:click={() => invoke("open_url", { url: "https://github.com/nekename" })} class="underline">nekename</button>
			</span>
		</div>
	</svelte:fragment>
</Popup>
