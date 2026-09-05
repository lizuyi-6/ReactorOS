// Behavioral tests against the real TypeScript HTTP layer and Pinia store.
// esbuild is already a transitive Vite dependency; no new runtime dependency.
import { build } from 'esbuild';
import { mkdtemp, rm } from 'node:fs/promises';
import { resolve, join } from 'node:path';
import { pathToFileURL } from 'node:url';
import { test, beforeEach, after } from 'node:test';
import assert from 'node:assert/strict';
import { createPinia, setActivePinia } from 'pinia';

const dir = await mkdtemp(resolve('.legacy-audit-tests-'));
await build({ entryPoints: ['frontend/src/api/http.ts', 'frontend/src/stores/live.ts', 'frontend/src/api/index.ts'],
  outdir: dir, bundle: true, platform: 'node', format: 'esm', packages: 'external',
  define: { 'import.meta.env': '{}' }, outExtension: { '.js': '.mjs' }, logLevel: 'silent' });
globalThis.localStorage = { getItem: () => null, setItem() {}, removeItem() {} };
globalThis.window = { setTimeout, clearTimeout, location: { protocol: 'http:', host: 'localhost' } };
const http = await import(pathToFileURL(join(dir, 'api/http.mjs')).href);
const { useLiveStore } = await import(pathToFileURL(join(dir, 'stores/live.mjs')).href);
let logoutCount = 0;
beforeEach(() => { http.setAuthToken(null); logoutCount = 0; http.setUnauthorizedHandler(() => { logoutCount++; }); });
after(async () => { await rm(dir, { recursive: true, force: true }); });
const response = (body, status = 200) => new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });
const deferred = () => { let resolve; const promise = new Promise(r => { resolve = r; }); return { promise, resolve }; };
const never = () => new Promise(() => {});
async function bounded(promise) {
  let timer;
  try { return await Promise.race([promise, new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Error('test harness deadline: request did not settle')), 250);
  })]); } finally { clearTimeout(timer); }
}

test('raw realtime payload retains device metadata and its data property', async () => {
  const raw = { device_id: 'reactor_001', status: 'emergency_stop', data: { current_temp: 60 } };
  globalThis.fetch = async () => response(raw);
  assert.deepEqual(await http.request('/realtime'), raw);
});
test('actual envelopes unwrap and HTTP 204 stays null', async () => {
  globalThis.fetch = async () => response({ code: 200, message: 'ok', data: { value: 3 } });
  assert.deepEqual(await http.request('/envelope'), { value: 3 });
  globalThis.fetch = async () => new Response(null, { status: 204 });
  assert.equal(await http.request('/empty'), null);
});
test('late 401 from session A cannot log out session B', async () => {
  const pending = deferred(); http.setAuthToken('A');
  globalThis.fetch = () => pending.promise;
  const request = http.request('/private');
  await Promise.resolve(); http.setAuthToken('B');
  pending.resolve(response({ message: 'expired' }, 401));
  await assert.rejects(request, e => e.status === 401);
  assert.equal(logoutCount, 0); assert.equal(http.getAuthToken(), 'B');
});
test('logout/login with the same token still fences an old request', async () => {
  const pending = deferred(); http.setAuthToken('A');
  globalThis.fetch = () => pending.promise;
  const request = http.request('/private');
  await Promise.resolve(); http.setAuthToken(null); http.setAuthToken('A');
  pending.resolve(response({ message: 'old error' }, 401));
  await assert.rejects(request); assert.equal(logoutCount, 0);
});
test('unauthenticated login failure does not destroy another session', async () => {
  http.setAuthToken('valid'); globalThis.fetch = async () => response({ message: 'bad password' }, 401);
  await assert.rejects(http.request('/login', { auth: false })); assert.equal(logoutCount, 0);
});
test('401 for current authenticated session still invokes logout', async () => {
  http.setAuthToken('expired'); globalThis.fetch = async () => response({ message: 'expired' }, 401);
  await assert.rejects(http.request('/private')); assert.equal(logoutCount, 1);
});
test('deadline covers a transport that does not settle; no automatic retry', async () => {
  let calls = 0; globalThis.fetch = () => { calls++; return never(); };
  await assert.rejects(bounded(http.request('/control', { method: 'POST', timeoutMs: 10 })),
    e => e.status === 0 && /timed out/.test(e.message));
  assert.equal(calls, 1);
});
test('deadline also covers JSON response body', async () => {
  globalThis.fetch = async () => ({ ok: true, status: 200, text: never });
  await assert.rejects(bounded(http.request('/live', { timeoutMs: 10 })), e => /timed out/.test(e.message));
});
test('deadline also covers Blob response body', async () => {
  globalThis.fetch = async () => ({ ok: true, status: 200, blob: never });
  await assert.rejects(bounded(http.requestBlob('/export', { timeoutMs: 10 })), e => /timed out/.test(e.message));
});
test('allowFailure also handles response-stream failure', async () => {
  globalThis.fetch = async () => ({ ok: true, status: 200, text: async () => { throw new Error('stream broke'); } });
  assert.equal(await http.request('/live', { allowFailure: true }), null);
});
test('caller cancellation is honored without issuing a request', async () => {
  const controller = new AbortController(); controller.abort(new Error('cancelled'));
  let calls = 0; globalThis.fetch = () => { calls++; return never(); };
  await assert.rejects(bounded(http.request('/live', { signal: controller.signal, timeoutMs: 10 })), e => /cancelled/.test(e.message));
  assert.equal(calls, 0);
});
test('Blob errors use the same stale-session guard and error normalization', async () => {
  const pending = deferred(); http.setAuthToken('A'); globalThis.fetch = () => pending.promise;
  const request = http.requestBlob('/export'); await Promise.resolve(); http.setAuthToken('B');
  pending.resolve(response({ message: 'expired' }, 401));
  await assert.rejects(request, e => e.status === 401); assert.equal(logoutCount, 0);
});
test('unavailable live data never exposes a fallback measurement as current', () => {
  setActivePinia(createPinia()); const store = useLiveStore();
  store.applyLive({ runtime: { latest_sample: { temperature_c: 60, captured_at: '2026-01-01T00:00:00Z' } } });
  assert.equal(store.latestSample.temperature_c, 60);
  store.applyLive(null); assert.equal(store.liveStatus, 'unavailable'); assert.equal(store.latestSample, null);
  assert.ok(store.runtimeFallback); // context retained, but not advertised as current
});
test('WebSocket updates safety flags and does not duplicate the same sample', () => {
  setActivePinia(createPinia()); const store = useLiveStore();
  let sock;
  globalThis.WebSocket = class {
    constructor() { sock = this; } close() {}
  };
  store.applyLive({ runtime: { emergency_stop: false, auto_enabled: true, latest_sample: null }, recent_samples: [] });
  store.bindTokenProvider(() => 'test'); store.connectRealtimeSocket();
  const payload = { device_id: 'reactor_001', timestamp: '2026-01-01T00:00:00Z', status: 'stopped', device_online: true,
    runtime: { emergency_stop: true, auto_enabled: false, last_control_error: 'fault' }, data: { current_temp: 60 } };
  try {
    sock.onmessage({ data: JSON.stringify(payload) }); sock.onmessage({ data: JSON.stringify(payload) });
    assert.equal(store.runtime.emergency_stop, true); assert.equal(store.runtime.auto_enabled, false);
    assert.equal(store.runtime.last_control_error, 'fault'); assert.equal(store.recentSamples.length, 1);
  } finally { store.disconnectRealtimeSocket(); }
});

test('AI operations retain a finite provider-compatible budget; telemetry remains short', async () => {
  const { aiApi, batchApi, systemApi, AI_REQUEST_TIMEOUT_MS } = await import(pathToFileURL(join(dir, 'api/index.mjs')).href);
  const originalTimer = globalThis.setTimeout;
  const budgets = [];
  let calls = 0;
  globalThis.setTimeout = (fn, delay, ...args) => { budgets.push(delay); return originalTimer(fn, delay, ...args); };
  globalThis.fetch = async () => { calls++; return response({ code: 200, data: {} }); };
  try {
    await aiApi.regenerateRecommendation();
    await aiApi.control({ dry_run: true });
    await aiApi.experimentPlan();
    await batchApi.saveProductResult({ batch_id: 1, yield_percent: 80, product_ratio: 0.8 });
    await systemApi.live();
    assert.equal(AI_REQUEST_TIMEOUT_MS, 90_000);
    assert.deepEqual(budgets, [90_000, 90_000, 90_000, 90_000, 15_000]);
    assert.equal(calls, 5);
  } finally { globalThis.setTimeout = originalTimer; }
});
