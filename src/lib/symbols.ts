// Built-in symbols for the box editor. Each is a 24x24 stroke icon; a symbol is stored on a key
// as an SVG data URL (image_override), marked so the editor can tell symbol / word / picture apart.

export const SYMBOLS: { [name: string]: string } = {
	"mic": "M12 2a3 3 0 0 0-3 3v7a3 3 0 0 0 6 0V5a3 3 0 0 0-3-3zM19 10v2a7 7 0 0 1-14 0v-2M12 19v3",
	"mic-off": "M2 2l20 20M9 9v3a3 3 0 0 0 5.1 2.1M15 9.3V5a3 3 0 0 0-5.9-.7M19 10v2a7 7 0 0 1-.9 3.4M5 10v2a7 7 0 0 0 12 5M12 19v3",
	"headphones": "M3 18v-6a9 9 0 0 1 18 0v6M21 19a2 2 0 0 1-2 2h-1v-6h3zM3 19a2 2 0 0 0 2 2h1v-6H3z",
	"volume": "M11 5 6 9H2v6h4l5 4V5zM15.5 8.5a5 5 0 0 1 0 7M19 5a10 10 0 0 1 0 14",
	"volume-off": "M11 5 6 9H2v6h4l5 4V5zM22 9l-6 6M16 9l6 6",
	"play": "M7 4l13 8-13 8z",
	"pause": "M7 4h3v16H7zM14 4h3v16h-3z",
	"next": "M5 4l10 8-10 8zM19 5v14",
	"previous": "M19 20 9 12l10-8zM5 19V5",
	"record": "M12 6a6 6 0 1 0 0 12 6 6 0 1 0 0-12z",
	"camera": "M4 7h3l2-3h6l2 3h3v13H4zM12 9a4 4 0 1 0 0 8 4 4 0 1 0 0-8z",
	"video": "M2 6h13v12H2zM15 10l7-4v12l-7-4",
	"screen": "M2 4h20v13H2zM8 21h8M12 17v4",
	"youtube": "M2 9a4 4 0 0 1 4-4h12a4 4 0 0 1 4 4v6a4 4 0 0 1-4 4H6a4 4 0 0 1-4-4zM10 9l5 3-5 3z",
	"chat": "M21 12a8 8 0 0 1-11.6 7.1L3 21l1.9-6.4A8 8 0 1 1 21 12z",
	"globe": "M12 2a10 10 0 1 0 0 20 10 10 0 1 0 0-20zM2 12h20M12 2a15 15 0 0 1 0 20M12 2a15 15 0 0 0 0 20",
	"folder": "M3 6h6l2 2h10v11H3z",
	"timer": "M12 5a8 8 0 1 0 0 16 8 8 0 1 0 0-16zM12 9v4l2 2M9 2h6",
	"cpu": "M6 6h12v12H6zM9 2v4M15 2v4M9 18v4M15 18v4M2 9h4M2 15h4M18 9h4M18 15h4",
	"bulb": "M9 18h6M10 22h4M12 2a7 7 0 0 0-4 12.7V16h8v-1.3A7 7 0 0 0 12 2z",
	"bolt": "M13 2 4 14h7l-1 8 9-12h-7z",
	"star": "M12 2l3 7 7 .6-5.3 4.7L18.2 22 12 18l-6.2 4 1.5-7.7L2 9.6 9 9z",
	"heart": "M12 21s-8-5-8-11a4.5 4.5 0 0 1 8-3 4.5 4.5 0 0 1 8 3c0 6-8 11-8 11z",
	"home": "M3 11l9-8 9 8M5 10v10h14V10",
	"lock": "M5 11h14v10H5zM8 11V7a4 4 0 0 1 8 0v4",
	"power": "M12 2v10M18.4 6.6a9 9 0 1 1-12.8 0",
	"settings": "M12 9a3 3 0 1 0 0 6 3 3 0 1 0 0-6zM12 2v3M12 19v3M2 12h3M19 12h3M4.9 4.9 7 7M17 17l2.1 2.1M4.9 19.1 7 17M17 7l2.1-2.1",
	"keyboard": "M2 6h20v12H2zM6 10h.01M10 10h.01M14 10h.01M18 10h.01M7 14h10",
	"copy": "M8 8h12v12H8zM4 16V4h12",
	"scissors": "M6 3a3 3 0 1 0 0 6 3 3 0 1 0 0-6zM6 15a3 3 0 1 0 0 6 3 3 0 1 0 0-6zM20 4 8.1 15.9M14.5 14.5 20 20M8.1 8.1 12 12",
	"undo": "M9 14 4 9l5-5M4 9h11a5 5 0 0 1 0 10h-3",
	"check": "M4 12l5 5L20 6",
	"plus": "M12 5v14M5 12h14",
	"arrow-left": "M15 6l-6 6 6 6",
	"arrow-right": "M9 6l6 6-6 6",
	"game": "M6 11h4M8 9v4M15 12h.01M18 10h.01M17.3 5H6.7a4 4 0 0 0-4 3.6L2 15a3 3 0 0 0 5.2 2L9 15h6l1.8 2a3 3 0 0 0 5.2-2l-.7-6.4a4 4 0 0 0-4-3.6z",
};

export const SYMBOL_COLOURS = ["#FFFFFF", "#FF6B6B", "#FF8A3D", "#FFD23D", "#3DDC97", "#4F8CFF", "#B57BFF"];
export const BOX_COLOURS = ["#000000", "#1A1E26", "#5865F2", "#E5484D", "#1DB954", "#FF8A3D", "#7C3AED"];

const MARK_SYMBOL = "orvo-symbol:";
const MARK_BLANK = "orvo-blank";

export function symbolUrl(name: string, colour: string): string {
	const d = SYMBOLS[name] ?? SYMBOLS["star"];
	const svg =
		`<svg xmlns="http://www.w3.org/2000/svg" width="144" height="144" viewBox="0 0 24 24"><!--${MARK_SYMBOL}${name}:${colour}-->` +
		`<path d="${d}" fill="none" stroke="${colour}" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>`;
	return "data:image/svg+xml;base64," + btoa(svg);
}

export function blankUrl(): string {
	const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="144" height="144"><!--${MARK_BLANK}--></svg>`;
	return "data:image/svg+xml;base64," + btoa(svg);
}

export type BoxMode = "live" | "symbol" | "word" | "picture";

type ModeInfo = { mode: BoxMode; symbol?: string; colour?: string };

function parse(svgText: string): ModeInfo | null {
	const m = svgText.match(/orvo-symbol:([a-z-]+):(#[0-9A-Fa-f]{6})/);
	if (m) return { mode: "symbol", symbol: m[1], colour: m[2] };
	if (svgText.includes(MARK_BLANK)) return { mode: "word" };
	return null;
}

// Quick check from a data URL (right after editing).
export function modeOf(image_override: string | null): ModeInfo {
	if (!image_override) return { mode: "live" };
	if (image_override.startsWith("data:image/svg+xml")) {
		const body = image_override.split(",")[1] ?? "";
		let text = "";
		try {
			text = image_override.includes(";base64,") ? atob(body) : decodeURIComponent(body);
		} catch {}
		return parse(text) ?? { mode: "picture" };
	}
	return { mode: "picture" };
}

// After a restart the app stores images as files, so read the saved SVG to find the symbol.
export async function modeOfAsync(image_override: string | null, url: (path: string) => string): Promise<ModeInfo> {
	const quick = modeOf(image_override);
	if (!image_override || image_override.startsWith("data:") || !image_override.toLowerCase().endsWith(".svg")) return quick;
	try {
		const text = await (await fetch(url(image_override))).text();
		return parse(text) ?? quick;
	} catch {
		return quick;
	}
}
