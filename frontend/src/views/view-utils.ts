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
