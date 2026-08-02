// 展示格式化工具。

export function fixed(value: number | null | undefined, digits = 1, suffix = ""): string {
  if (value === null || value === undefined || !Number.isFinite(value)) return "--";
  return `${value.toFixed(digits)}${suffix}`;
}

export function text(value: unknown, fallback = "--"): string {
  if (value === null || value === undefined || value === "") return fallback;
  return String(value);
}

/** 把 ISO-8601（含纳秒）时间戳格式化为本地 "YYYY-MM-DD HH:mm:ss"。 */
export function formatTimestamp(value: unknown, fallback = "--"): string {
  if (value === null || value === undefined || value === "") return fallback;
  const date = new Date(String(value));
  if (Number.isNaN(date.getTime())) return fallback;
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}`;
}

export function formatTime(value: unknown, fallback = "--"): string {
  if (value === null || value === undefined || value === "") return fallback;
  const date = new Date(String(value));
  if (Number.isNaN(date.getTime())) return fallback;
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}`;
}

export function boolText(value: unknown): boolean {
  if (typeof value === "boolean") return value;
  if (typeof value === "string") return value.toLowerCase() === "true";
  if (typeof value === "number") return value !== 0;
  return false;
}
