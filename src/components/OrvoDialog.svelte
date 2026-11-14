<script lang="ts">
	import { closeDialog, message, openDialog } from "$lib/dialog";
	import { invoke } from "@tauri-apps/api/core";
	import { onMount, tick } from "svelte";

	// "Welcome" / "updated to vX" note from startup
	onMount(async () => {
		try {
			const notice = await invoke<[string, string] | null>("orvo_take_startup_notice");
			if (notice) message(notice[1], { title: notice[0] });
		} catch {}
	});

	let okButton: HTMLButtonElement;
	$: if ($openDialog) tick().then(() => okButton?.focus());

	function key(e: KeyboardEvent) {
		if (!$openDialog) return;
		if (e.key == "Escape") {
			e.preventDefault();
			e.stopPropagation();
			closeDialog(false);
		} else if (e.key == "Enter") {
			e.preventDefault();
			e.stopPropagation();
			closeDialog(true);
		}
	}
</script>

<svelte:window on:keydown|capture={key} />

{#if $openDialog}
	<div class="fixed inset-0 z-[100] flex items-center justify-center bg-black/55 backdrop-blur-[2px]" role="presentation" on:click|self={() => closeDialog(false)}>
		<div
			class="w-[420px] max-w-[90vw] rounded-2xl bg-neutral-800 border border-neutral-700 shadow-2xl p-5"
			role="alertdialog"
			aria-modal="true"
			aria-labelledby="orvo-dialog-title"
			aria-describedby="orvo-dialog-text"
		>
			<div class="flex items-center gap-2.5 mb-2">
				<span
					class="w-2.5 h-2.5 rounded-full shrink-0"
					style="background: {$openDialog.kind == 'error' ? '#f87171' : $openDialog.kind == 'warning' || $openDialog.danger ? 'var(--orvo-orange)' : 'var(--orvo-accent)'}"
				></span>
				<h2 id="orvo-dialog-title" class="text-base font-semibold text-neutral-100">{$openDialog.title}</h2>
			</div>
			<p id="orvo-dialog-text" class="text-sm text-neutral-300 whitespace-pre-line leading-relaxed">{$openDialog.text}</p>
			<div class="flex justify-end gap-2 mt-5">
				{#if $openDialog.cancel != null}
					<button class="px-4 h-9 rounded-lg text-sm text-neutral-200 bg-neutral-700 hover:bg-neutral-600 border border-neutral-600" on:click={() => closeDialog(false)}>
						{$openDialog.cancel}
					</button>
				{/if}
				<button
					bind:this={okButton}
					class="px-4 h-9 rounded-lg text-sm font-semibold text-white hover:brightness-110"
					style="background: {$openDialog.danger && $openDialog.cancel != null ? '#dc2626' : 'var(--orvo-accent)'}"
					on:click={() => closeDialog(true)}
				>
					{$openDialog.ok}
				</button>
			</div>
		</div>
	</div>
{/if}
