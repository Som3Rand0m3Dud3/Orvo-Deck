<script lang="ts">
	export let icon: string;
	export let name: string;
	export let subtitle: string;
	export let hidden: boolean = false;
	export let disconnected: boolean = false;
	export let action: () => void;
	export let actionLabel: string = "";
	export let secondaryAction: (() => void) | undefined = undefined;
	export let secondaryActionLabel: string = "";

	// Missing or broken icons get a letter tile instead of the browser's broken-image text.
	let failed = false;
	let lastIcon = icon;
	$: if (icon != lastIcon) {
		lastIcon = icon;
		failed = false;
	}
	$: letter = (name.match(/[\p{L}\p{N}]/u)?.[0] ?? "?").toUpperCase();
</script>

<div class="flex flex-row items-center m-2 p-2 bg-neutral-700 hover:bg-neutral-600 transition-colors border border-neutral-600 rounded-lg" class:hidden>
	{#if icon && !icon.endsWith("/") && !failed}
		<img src={icon} class="w-24 h-24 shrink-0 rounded-lg" class:opacity-75={disconnected} alt="" loading="lazy" on:error={() => (failed = true)} />
	{:else}
		<div
			class="flex items-center justify-center w-24 h-24 shrink-0 rounded-lg border border-neutral-600 text-4xl font-bold text-neutral-100 select-none"
			class:opacity-75={disconnected}
			style="background: linear-gradient(135deg, color-mix(in srgb, var(--orvo-accent) 55%, #171717), #262626)"
			aria-hidden="true"
		>
			{letter}
		</div>
	{/if}
	<div class="ml-4 mr-2 text-neutral-300 wrap-anywhere" class:opacity-75={disconnected}>
		<p class="font-semibold">{name}</p>
		<slot name="subtitle">{subtitle}</slot>
	</div>

	<div class="flex flex-col ml-auto mr-4">
		{#if secondaryAction}
			<button on:click={secondaryAction} aria-label={secondaryActionLabel}>
				<slot name="secondary" />
			</button>
		{/if}
		<button on:click={action} aria-label={actionLabel}>
			<slot />
		</button>
	</div>
</div>
