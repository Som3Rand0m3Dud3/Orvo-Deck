/// <reference lib="deno.ns" />
// deno-lint-ignore-file no-import-prefix

// Builds the NineDeck device plugin (Go) for the target being built.

import { copy } from "jsr:@std/fs@^1.0";
import { join } from "jsr:@std/path@^1.0";

if (Deno.args.length < 2) Deno.exit(1);
const outDir = Deno.args[0];
const target = Deno.args[1];

try {
	await Deno.remove(outDir, { recursive: true });
} catch (error: any) {
	if (!(error instanceof Deno.errors.NotFound)) {
		throw error;
	}
}

await copy("assets", outDir);

let goos = "linux";
let output = "ninedeck-linux";
if (target.includes("windows")) {
	goos = "windows";
	output = "ninedeck-win.exe";
} else if (target.includes("apple")) {
	goos = "darwin";
	output = "ninedeck-mac";
}
const goarch = target.startsWith("aarch64") ? "arm64" : "amd64";
const env = { GOOS: goos, GOARCH: goarch, CGO_ENABLED: "0" };

async function run(args: string[]) {
	const status = await new Deno.Command("go", { args, cwd: "src", env }).spawn().status;
	if (!status.success) Deno.exit(1);
}

await run(["mod", "tidy"]);
const ldflags = goos == "windows" ? "-s -w -H windowsgui" : "-s -w";
await run(["build", "-ldflags", ldflags, "-o", join(outDir, output), "."]);
