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
  /** Total deadline, including response-body consumption. No automatic retries. */
  timeoutMs?: number;
}

interface Envelope {
  code?: number;
  message?: string;
  data?: unknown;
}

export const DEFAULT_REQUEST_TIMEOUT_MS = 15_000;
let authEpoch = 0;
let authToken: string | null = null;
let onUnauthorized: (() => void) | null = null;

export function setAuthToken(token: string | null): void {
  if (authToken !== token) authEpoch += 1;
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
  if (payload && typeof payload === "object"
      && typeof (payload as Envelope).code === "number" && "data" in payload) {
    return (payload as Envelope).data as T;
  }
  return payload as T;
}

// One lifetime for headers AND body; racing also bounds non-cooperative mocks
// or transports. Aborting a POST does not prove the server cancelled the action.
async function withDeadline<T>(options: RequestOptions, run: (signal: AbortSignal) => Promise<T>): Promise<T> {
  const timeoutMs = options.timeoutMs ?? DEFAULT_REQUEST_TIMEOUT_MS;
  if (!Number.isFinite(timeoutMs) || timeoutMs <= 0 || timeoutMs > 2_147_483_647) {
    throw new Error("timeoutMs must be a positive finite timer duration");
  }
  const controller = new AbortController();
  const forwardAbort = () => controller.abort(options.signal?.reason);
  const onTimeout = () => controller.abort(new Error("Request timed out; a submitted action may still have taken effect. Refresh device state before retrying."));
  let rejectAbort: (reason?: unknown) => void = () => {};
  const aborted = new Promise<never>((_, reject) => { rejectAbort = reject; });
  const handleAbort = () => rejectAbort(controller.signal.reason ?? new Error("Request aborted"));
  controller.signal.addEventListener("abort", handleAbort, { once: true });
  options.signal?.addEventListener("abort", forwardAbort, { once: true });
  const timer = setTimeout(onTimeout, timeoutMs);
  if (options.signal?.aborted) forwardAbort();
  try {
    return await Promise.race([
      aborted,
      Promise.resolve().then(() => {
        controller.signal.throwIfAborted();
        return run(controller.signal);
      })
    ]);
  } finally {
    clearTimeout(timer);
    options.signal?.removeEventListener("abort", forwardAbort);
    controller.signal.removeEventListener("abort", handleAbort);
  }
}

async function performRequest<T>(
  path: string,
  options: RequestOptions,
  consume: (response: Response) => Promise<T>,
  accept: string
): Promise<T> {
  const sentToken = options.auth === false ? null : authToken;
  const sentEpoch = authEpoch;
  try {
    const headers = new Headers();
    headers.set("Accept", options.accept ?? accept);
    const body = serializeBody(options.body);
    if (body !== undefined) headers.set("Content-Type", "application/json");
    if (sentToken) headers.set("Authorization", `Bearer ${sentToken}`);
    return await withDeadline(options, async (signal) => {
      const response = await fetch(path, {
        method: options.method ?? (options.body === undefined ? "GET" : "POST"),
        headers, body, cache: "no-store", signal
      });
      signal.throwIfAborted();
      // A late 401 from session A must never log out a newly established B.
      if (response.status === 401 && sentToken && sentToken === authToken && sentEpoch === authEpoch) {
        onUnauthorized?.();
      }
      if (!response.ok) {
        const payload = await parsePayload(response);
        const err: ApiError = {
          status: response.status,
          message: extractErrorMessage(payload, `${response.status} ${response.statusText}`),
          payload
        };
        throw err;
      }
      return consume(response);
    });
  } catch (error) {
    if (options.allowFailure) return null as T;
    throw toApiError(error);
  }
}

export function request<T>(path: string, options: RequestOptions = {}): Promise<T> {
  return performRequest(path, options, async (response) => unwrap<T>(await parsePayload(response)), "application/json");
}

export function requestBlob(path: string, options: RequestOptions = {}): Promise<Blob> {
  return performRequest(path, options, (response) => response.blob(), "application/octet-stream");
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
