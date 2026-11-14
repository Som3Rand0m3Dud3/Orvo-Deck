// Orvo-styled pop-ups (used instead of Windows' own message boxes). Same calls as
// @tauri-apps/plugin-dialog's ask() and message(), shown by <OrvoDialog /> inside the app.
import { writable } from "svelte/store";

type Kind = "info" | "warning" | "error";
type Buttons = { ok?: string; cancel?: string; yes?: string; no?: string } | string;
export type DialogOptions = { title?: string; kind?: Kind; okLabel?: string; cancelLabel?: string; buttons?: Buttons };
export type OpenDialog = { text: string; title: string; kind: Kind; ok: string; cancel: string | null; danger: boolean; resolve: (yes: boolean) => void };

export const openDialog = writable<OpenDialog | null>(null);
const queue: OpenDialog[] = [];

function show(d: OpenDialog) {
	queue.push(d);
	if (queue.length == 1) openDialog.set(d);
}

export function closeDialog(yes: boolean) {
	const d = queue.shift();
	d?.resolve(yes);
	openDialog.set(queue[0] ?? null);
}

function label(b: Buttons | undefined, key: "ok" | "cancel" | "yes" | "no"): string | undefined {
	return typeof b == "object" ? b[key] : undefined;
}

export function message(text: string, options?: DialogOptions | string): Promise<void> {
	const o: DialogOptions = typeof options == "string" ? { title: options } : options ?? {};
	return new Promise((resolve) =>
		show({ text, title: o.title ?? "Orvo Studio", kind: o.kind ?? "info", ok: o.okLabel ?? label(o.buttons, "ok") ?? "OK", cancel: null, danger: false, resolve: () => resolve() })
	);
}

export function ask(text: string, options?: DialogOptions | string): Promise<boolean> {
	const o: DialogOptions = typeof options == "string" ? { title: options } : options ?? {};
	const title = o.title ?? "Orvo Studio";
	const danger = o.kind == "warning" || /delete|remove|uninstall/i.test(title + " " + text.slice(0, 40));
	return new Promise((resolve) =>
		show({
			text,
			title,
			kind: o.kind ?? "info",
			ok: o.okLabel ?? label(o.buttons, "yes") ?? label(o.buttons, "ok") ?? (/delete/i.test(title) ? "Delete" : "Yes"),
			cancel: o.cancelLabel ?? label(o.buttons, "no") ?? label(o.buttons, "cancel") ?? (/delete/i.test(title) ? "Cancel" : "No"),
			danger,
			resolve,
		})
	);
}
