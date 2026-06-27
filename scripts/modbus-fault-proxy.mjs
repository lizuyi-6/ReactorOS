#!/usr/bin/env node
// Modbus TCP fault-injecting transparent TCP proxy.
// Usage:
//   node scripts/modbus-fault-proxy.mjs \
//     --listen 0.0.0.0:5502 --upstream 127.0.0.1:502 \
//     --loss-pct 5 --delay-ms 50 --bit-flip-pct 0
//
// The proxy is intentionally minimal: it forwards Modbus TCP request/response
// frames byte-by-byte. loss-pct and bit-flip-pct operate per-direction on
// each forwarded chunk. delay-ms adds a uniform one-way delay.
//
// Used by:
//   docs/upper_computer_external_acceptance_checklist.md
//   docs/upper_computer_rk_deployment_acceptance_guide.md
import net from "node:net";

function parseArgs(argv) {
  const out = {};
  for (let i = 2; i < argv.length; i++) {
    const flag = argv[i];
    if (!flag.startsWith("--")) continue;
    const key = flag.slice(2);
    const val = argv[i + 1];
    out[key] = val;
    i++;
  }
  return out;
}

const args = parseArgs(process.argv);
const listen = (args.listen ?? "0.0.0.0:5502").split(":");
const upstream = (args.upstream ?? "127.0.0.1:502").split(":");
const lossPct = Number(args["loss-pct"] ?? 0);
const delayMs = Number(args["delay-ms"] ?? 0);
const bitFlipPct = Number(args["bit-flip-pct"] ?? 0);

if (listen.length !== 2 || upstream.length !== 2) {
  process.stderr.write("listen/upstream must be host:port\n");
  process.exit(1);
}

const port = Number(listen[1]);
const upHost = upstream[0];
const upPort = Number(upstream[1]);

function pickDrop() {
  return Math.random() * 100 < lossPct;
}
function maybeFlip(buf) {
  if (bitFlipPct <= 0) return buf;
  const arr = Buffer.from(buf);
  for (let i = 0; i < arr.length; i++) {
    if (Math.random() * 100 < bitFlipPct) {
      arr[i] = arr[i] ^ (1 << Math.floor(Math.random() * 8));
    }
  }
  return arr;
}
function forwardChunks(src, dst, label) {
  src.on("data", (chunk) => {
    if (pickDrop()) {
      process.stdout.write(`[${label}] dropped ${chunk.length} bytes\n`);
      return;
    }
    let payload = maybeFlip(chunk);
    if (delayMs > 0) {
      setTimeout(() => {
        if (!dst.destroyed) dst.write(payload);
      }, delayMs);
    } else if (!dst.destroyed) {
      dst.write(payload);
    }
  });
  src.on("end", () => dst.end());
  src.on("error", (err) => process.stderr.write(`[${label}] ${err.message}\n`));
  src.on("close", () => dst.end());
}

const server = net.createServer((client) => {
  process.stdout.write(`[proxy] client connected ${client.remoteAddress}:${client.remotePort}\n`);
  const upstream = net.connect(upPort, upHost, () => {
    process.stdout.write(`[proxy] upstream connected ${upHost}:${upPort}\n`);
  });
  forwardChunks(client, upstream, "c->u");
  forwardChunks(upstream, client, "u->c");
});

server.listen(port, listen[0], () => {
  process.stdout.write(
    `modbus-fault-proxy listening on ${listen[0]}:${port} -> ${upHost}:${upPort} ` +
      `(loss=${lossPct}% delay=${delayMs}ms flip=${bitFlipPct}%)\n`
  );
});
