import type { ApiRecord } from "../stores/plant";

export function objectAt(source: unknown, key: string): ApiRecord | null {
  if (!source || typeof source !== "object") return null;
  const value = (source as ApiRecord)[key];
  return value && typeof value === "object" ? (value as ApiRecord) : null;
}

export function arrayAt<T = ApiRecord>(source: unknown, key: string): T[] {
  if (!source || typeof source !== "object") return [];
  const value = (source as ApiRecord)[key];
  return Array.isArray(value) ? (value as T[]) : [];
}

export function numberAt(source: unknown, key: string): number | null {
  if (!source || typeof source !== "object") return null;
  const value = (source as ApiRecord)[key];
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

export function textAt(source: unknown, key: string, fallback = "--"): string {
  if (!source || typeof source !== "object") return fallback;
  const value = (source as ApiRecord)[key];
  if (value === null || value === undefined || value === "") return fallback;
  return String(value);
}

// Format an ISO-8601 timestamp (e.g. backend "2026-06-13T15:26:28.179151100Z",
// with nanosecond precision) into a concise local time for operators. Returns
// the fallback for empty/invalid input so the column never shows a raw ISO
// string or "Invalid Date".
export function formatTimestamp(value: unknown, fallback = "--"): string {
  if (value === null || value === undefined || value === "") return fallback;
  const text = String(value);
  const date = new Date(text);
  if (Number.isNaN(date.getTime())) return fallback;
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}`;
}

export function fixed(value: number | null, digits = 1, suffix = ""): string {
  return value === null ? "--" : `${value.toFixed(digits)}${suffix}`;
}

export function latestSample(live: ApiRecord | null): ApiRecord | null {
  const runtime = objectAt(live, "runtime");
  return objectAt(runtime, "latest_sample");
}

export function recentSamples(live: ApiRecord | null): ApiRecord[] {
  return arrayAt(live, "recent_samples");
}
