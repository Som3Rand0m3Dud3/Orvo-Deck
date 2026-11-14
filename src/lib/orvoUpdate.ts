import { writable } from "svelte/store";

// Newest Orvo Studio version found by the background check (null = up to date / not checked).
export const appUpdate = writable<{ tag: string } | null>(null);
