#!/usr/bin/env node
// AINAS mock server.
//
// Speaks the subset of the AINAS REST contract that the reactor-edge
// daemon actually calls (POST /tasks, GET /tasks/:id, GET /tasks?limit=N).
// Tasks transition pending -> executed/rejected within one poll cycle and
// a Server-Sent-Events stream on /tasks/:id/events emits a "receipt"
// notification so the daemon's mqtt task bridge has something to publish.
//
// Usage:
//   node scripts/mocks/ainas-mock-server.mjs --listen 127.0.0.1:5599
//
// The mock is dependency-free.

import http from "node:http";
import { randomUUID } from "node:crypto";

function parseArgs(argv) {
  const out = {};
  for (let i = 2; i < argv.length; i++) {
    const flag = argv[i];
    if (!flag.startsWith("--")) continue;
    out[flag.slice(2)] = argv[i + 1];
    i++;
  }
  return out;
}

const args = parseArgs(process.argv);
const [host, portText] = (args.listen ?? "127.0.0.1:5599").split(":");
const port = Number(portText);

const tasks = new Map();
let nextId = 1;

function jsonResponse(res, status, body) {
  const payload = JSON.stringify(body);
  res.writeHead(status, {
    "content-type": "application/json; charset=utf-8",
    "content-length": Buffer.byteLength(payload)
  });
  res.end(payload);
}

function readJson(req) {
  return new Promise((resolve, reject) => {
    let raw = "";
    req.on("data", (chunk) => (raw += chunk));
    req.on("end", () => {
      if (!raw) return resolve({});
      try {
        resolve(JSON.parse(raw));
      } catch (err) {
        reject(err);
      }
    });
    req.on("error", reject);
  });
}

const server = http.createServer(async (req, res) => {
  try {
    const url = new URL(req.url, `http://${req.headers.host}`);
    if (req.method === "GET" && url.pathname === "/health") {
      return jsonResponse(res, 200, { status: "ok", tasks: tasks.size });
    }
    if (req.method === "GET" && url.pathname === "/tasks") {
      const limit = Math.max(1, Math.min(200, Number(url.searchParams.get("limit") ?? 50)));
      const list = Array.from(tasks.values()).slice(-limit);
      return jsonResponse(res, 200, { data: list });
    }
    const createMatch = req.method === "POST" && url.pathname === "/tasks";
    if (createMatch) {
      const body = await readJson(req);
      const id = String(nextId++);
      const now = new Date().toISOString();
      const task = {
        id,
        external_task_id: body.external_task_id ?? null,
        intent: body.intent ?? "set_targets",
        status: "pending",
        request_json: body,
        response_json: null,
        created_at: now,
        updated_at: now
      };
      tasks.set(id, task);
      // Simulate the real AINAS round-trip: complete the task on next tick.
      setImmediate(() => {
        const completed = { ...task, status: "executed", response_json: { echoed: body, completed_by: "ainas-mock" }, updated_at: new Date().toISOString() };
        tasks.set(id, completed);
      });
      return jsonResponse(res, 201, { data: task });
    }
    const idMatch = url.pathname.match(/^\/tasks\/([^/]+)$/);
    if (idMatch && req.method === "GET") {
      const t = tasks.get(idMatch[1]);
      if (!t) return jsonResponse(res, 404, { error: "not found" });
      return jsonResponse(res, 200, { data: t });
    }
    const eventMatch = url.pathname.match(/^\/tasks\/([^/]+)\/events$/);
    if (eventMatch && req.method === "GET") {
      // Minimal SSE: one event, then close.
      res.writeHead(200, {
        "content-type": "text/event-stream",
        "cache-control": "no-cache",
        connection: "keep-alive"
      });
      const t = tasks.get(eventMatch[1]);
      res.write(`event: receipt\ndata: ${JSON.stringify(t ?? { id: eventMatch[1] })}\n\n`);
      res.end();
      return;
    }
    return jsonResponse(res, 404, { error: "not found" });
  } catch (err) {
    return jsonResponse(res, 500, { error: err instanceof Error ? err.message : String(err) });
  }
});

server.listen(port, host, () => {
  process.stdout.write(`ainas-mock listening on http://${host}:${port}\n`);
});
