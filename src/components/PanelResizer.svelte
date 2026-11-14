<script lang="ts">
	// A vertical grab bar on a side panel's inner edge. Drag to resize, double-click to reset.
	export let width: number;
	export let min: number;
	export let max: number;
	export let initial: number;
	/** Which edge of the panel the bar sits on: dragging toward the panel's outside makes it narrower. */
	export let edge: "left" | "right";
	export let storageKey: string;
	export let label: string;

	try {
		const saved = Number(localStorage.getItem(storageKey));
		if (saved) width = Math.min(max, Math.max(min, saved));
	} catch {}

	const save = () => {
		try {
			localStorage.setItem(storageKey, String(Math.round(width)));
		} catch {}
	};

	let startX = 0;
	let startW = 0;
	let dragging = false;

	function down(event: PointerEvent) {
		if (event.button != 0) return;
		startX = event.clientX;
		startW = width;
		dragging = true;
		(event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
		document.body.classList.add("orvo-resizing");
		event.preventDefault();
	}
	function move(event: PointerEvent) {
		if (!dragging) return;
		const dx = event.clientX - startX;
		width = Math.min(max, Math.max(min, startW + (edge == "right" ? dx : -dx)));
	}
	function up() {
		if (!dragging) return;
		dragging = false;
		document.body.classList.remove("orvo-resizing");
		save();
	}
	function key(event: KeyboardEvent) {
		const step = event.shiftKey ? 40 : 10;
		const grow = edge == "right" ? "ArrowRight" : "ArrowLeft";
		const shrink = edge == "right" ? "ArrowLeft" : "ArrowRight";
		if (event.key == grow) width = Math.min(max, width + step);
		else if (event.key == shrink) width = Math.max(min, width - step);
		else return;
		event.preventDefault();
		save();
	}
</script>

<!-- svelte-ignore a11y-no-noninteractive-tabindex a11y-no-noninteractive-element-interactions -->
<div
	role="separator"
	aria-orientation="vertical"
	aria-label={label}
	aria-valuenow={Math.round(width)}
	aria-valuemin={min}
	aria-valuemax={max}
	tabindex="0"
	title="Drag to resize · double-click to reset"
	class="group absolute top-0 bottom-0 w-2 z-20 cursor-col-resize outline-none {edge == 'right' ? '-right-1' : '-left-1'}"
	on:pointerdown={down}
	on:pointermove={move}
	on:pointerup={up}
	on:pointercancel={up}
	on:lostpointercapture={up}
	on:dblclick={() => {
		width = initial;
		save();
	}}
	on:keydown={key}
>
	<div
		class="mx-auto h-full w-0.5 transition-colors {dragging ? '' : 'bg-transparent group-hover:bg-neutral-500 group-focus-visible:bg-neutral-400'}"
		style={dragging ? "background: var(--orvo-accent)" : ""}
	></div>
</div>

<style>
	:global(body.orvo-resizing) {
		cursor: col-resize !important;
		user-select: none;
	}
	:global(body.orvo-resizing iframe) {
		pointer-events: none;
	}
</style>
