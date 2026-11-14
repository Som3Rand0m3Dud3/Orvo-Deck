import { writable } from "svelte/store";

// Orvo: size of everything in the bottom actions panel (Settings → Bottom panel size).
function load(): number {
	try {
		const v = parseFloat(localStorage.getItem("orvo-tray-scale") ?? "");
		if (v >= 0.75 && v <= 1.5) return v;
	} catch {}
	return 1;
}
export const trayScale = writable(load());
trayScale.subscribe((v) => {
	try {
		localStorage.setItem("orvo-tray-scale", String(v));
	} catch {}
});
