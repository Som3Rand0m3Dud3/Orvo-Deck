// Orvo Studio: runs on Elgato Marketplace pages inside Orvo's Marketplace window.
// It does nothing while you browse. Only when "Open in Stream Deck" is pressed does it talk to Orvo,
// by navigating to https://orvo-bridge.invalid/…, which Orvo catches and cancels.
(() => {
	if (window.__orvoBridge || location.hostname !== "marketplace.elgato.com") return;
	window.__orvoBridge = true;

	const send = (path, params) => {
		const url = new URL("https://orvo-bridge.invalid" + path);
		for (const [key, value] of Object.entries(params)) if (value != null) url.searchParams.set(key, String(value));
		location.href = url.toString();
	};

	const session = async () => {
		try {
			const response = await fetch("/api/auth/session", { credentials: "include" });
			return response.ok ? await response.json() : null;
		} catch {
			return null;
		}
	};

	// The page's data. The site changes pages without reloading, so read it fresh from the current page.
	const pageData = async () => {
		try {
			const html = await (await fetch(location.href, { credentials: "include" })).text();
			const match = html.match(/<script id="__NEXT_DATA__"[^>]*>([\s\S]*?)<\/script>/);
			if (match) return JSON.parse(match[1]);
		} catch {}
		return window.__NEXT_DATA__ || null;
	};

	let passThrough = false;
	const fallBack = (button, msg) => {
		send("/notice", { msg });
		// Let the Marketplace do its normal hand-off; Orvo catches that too.
		setTimeout(() => {
			passThrough = true;
			button.click();
			passThrough = false;
		}, 200);
	};

	document.addEventListener(
		"click",
		async (event) => {
			if (passThrough) return;
			const button = event.target instanceof Element ? event.target.closest("button, a") : null;
			if (!button || !/open in stream deck/i.test(button.textContent || "")) return;
			event.preventDefault();
			event.stopImmediatePropagation();

			const s = await session();
			const token = s && s.accessToken;
			const data = await pageData();
			const content = data && data.props && data.props.pageProps && data.props.pageProps.content;
			const productId = content && content.variants && content.variants[0] && content.variants[0].id;
			if (!token || !productId) return fallBack(button, "no session or product id");
			try {
				const response = await fetch(`https://mp-gateway.elgato.com/items/${productId}/purchase-link`, {
					headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
				});
				const json = await response.json();
				if (!json || !json.direct_link) return fallBack(button, "no download link");
				send("/install", { url: json.direct_link, name: content.name });
			} catch (error) {
				fallBack(button, String(error));
			}
		},
		true,
	);
})();
