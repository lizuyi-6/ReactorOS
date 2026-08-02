// 统一 HTTP 层：信封拆解、认证头、错误规范化、401 登出回调、Blob 下载。
// 后端约定（见 FRONTEND_REBUILD_AUDIT.md §2）：
// - 多数接口返回 {code,message,data}；约 10 个接口裸返回或 204。
// - 错误统一 {code,message,data:{error}}，HTTP 状态码与 code 一致。
// - 显式 null 字段会被 400，因此 body 序列化前剔除 undefined/null。

import type { ApiError } from "./errors";
import { toApiError } from "./errors";

export interface RequestOptions {
  method?: "GET" | "POST" | "PUT" | "DELETE";
  body?: unknown;
  /** 默认 true：携带 Bearer token。 */
  auth?: boolean;
  /** true 时任何失败都返回 null（用于可降级的聚合端点）。 */
  allowFailure?: boolean;
  accept?: string;
  signal?: AbortSignal;
}

interface Envelope {
  code?: number;
  message?: string;
  data?: unknown;
}

let authToken: string | null = null;
let onUnauthorized: (() => void) | null = null;

export function setAuthToken(token: string | null): void {
  authToken = token;
}

export function getAuthToken(): string | null {
  return authToken;
}

export function setUnauthorizedHandler(handler: (() => void) | null): void {
  onUnauthorized = handler;
}

function serializeBody(body: unknown): string | undefined {
  if (body === undefined) return undefined;
  if (body && typeof body === "object" && !Array.isArray(body)) {
    // 显式 null 会被后端 400（Option<Option<T>>），因此剔除 null/undefined 字段。
    const cleaned: Record<string, unknown> = {};
    for (const [key, value] of Object.entries(body as Record<string, unknown>)) {
      if (value !== undefined && value !== null) cleaned[key] = value;
    }
    return JSON.stringify(cleaned);
  }
  return JSON.stringify(body);
}

function extractErrorMessage(payload: unknown, fallback: string): string {
  if (!payload || typeof payload !== "object") return fallback;
  const record = payload as Record<string, unknown>;
  const data = record.data;
  if (data && typeof data === "object") {
    const err = (data as Record<string, unknown>).error;
    if (typeof err === "string" && err) return err;
  }
  if (typeof record.error === "string" && record.error) return record.error;
  if (typeof record.message === "string" && record.message) return record.message;
  return fallback;
}

async function parsePayload(response: Response): Promise<unknown> {
  const text = await response.text();
  if (!text) return null;
  try {
    return JSON.parse(text) as unknown;
  } catch {
    return { message: text };
  }
}

function unwrap<T>(payload: unknown): T {
  if (payload && typeof payload === "object" && "data" in (payload as Envelope)) {
    return (payload as Envelope).data as T;
  }
  return payload as T;
}

export async function request<T>(path: string, options: RequestOptions = {}): Promise<T> {
  const headers = new Headers();
  headers.set("Accept", options.accept ?? "application/json");
  const body = serializeBody(options.body);
  if (body !== undefined) headers.set("Content-Type", "application/json");
  if (options.auth !== false && authToken) headers.set("Authorization", `Bearer ${authToken}`);

  let response: Response;
  try {
    response = await fetch(path, {
      method: options.method ?? (options.body === undefined ? "GET" : "POST"),
      headers,
      body,
      cache: "no-store",
      signal: options.signal
    });
  } catch (error) {
    if (options.allowFailure) return null as T;
    throw toApiError(error);
  }

  const payload = await parsePayload(response);

  if (response.status === 401) {
    // token 缺失/过期/签名无效：触发统一登出。
    if (onUnauthorized) onUnauthorized();
  }

  if (!response.ok) {
    if (options.allowFailure) return null as T;
    const err: ApiError = {
      status: response.status,
      message: extractErrorMessage(payload, `${response.status} ${response.statusText}`),
      payload
    };
    throw err;
  }

  return unwrap<T>(payload);
}

export async function requestBlob(path: string, options: RequestOptions = {}): Promise<Blob> {
  const headers = new Headers();
  headers.set("Accept", options.accept ?? "application/octet-stream");
  if (options.auth !== false && authToken) headers.set("Authorization", `Bearer ${authToken}`);

  const response = await fetch(path, {
    method: options.method ?? "GET",
    headers,
    cache: "no-store",
    signal: options.signal
  });

  if (response.status === 401 && onUnauthorized) onUnauthorized();

  if (!response.ok) {
    const payload = await parsePayload(response);
    const err: ApiError = {
      status: response.status,
      message: extractErrorMessage(payload, `${response.status} ${response.statusText}`),
      payload
    };
    throw err;
  }
  return response.blob();
}

export function downloadBlob(blob: Blob, filename: string): void {
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = filename;
  document.body.appendChild(anchor);
  anchor.click();
  document.body.removeChild(anchor);
  URL.revokeObjectURL(url);
}
