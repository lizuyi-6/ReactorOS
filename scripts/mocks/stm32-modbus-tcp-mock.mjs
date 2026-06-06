#!/usr/bin/env node
// STM32 Modbus TCP mock server.
//
// Implements the smallest MBAP/PDU subset that xingshu-reactor-edge-daemon
// (modbus_tcp) and external tools (Modbus Poll/Slave) need to exercise the
// full register map. The mock keeps a register file in memory and accepts
// function codes 01/02/03/04/05/06/0F/10. Holding registers follow the
// address map declared in config/device.toml `[modbus.registers.*].address`.
//
// Usage:
//   node scripts/mocks/stm32-modbus-tcp-mock.mjs --listen 0.0.0.0:5502 \
//       --registers config/device.toml --fault drop-pct 0.0
//
// The mock is intentionally dependency-free: it parses MBAP/PDU bytes
// directly so the acceptance suite can run on machines without
// `modbus-serial` installed.

import fs from "node:fs";
import net from "node:net";
import path from "node:path";

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

function loadRegisterMap(filePath) {
  // Very small TOML-ish parser: only matches [modbus.registers.<name>] blocks
  // and reads `address`, `scale`, `offset`. The full daemon reads the real
  // TOML; this mock only needs address/scale so the response decodes right.
  const text = fs.readFileSync(filePath, "utf8");
  const lines = text.split(/\r?\n/);
  const registers = {};
  let currentName = null;
  for (const line of lines) {
    const m = line.match(/^\[modbus\.registers\.([a-z_]+)\]\s*$/);
    if (m) {
      currentName = m[1];
      registers[currentName] = { address: null, scale: 1, offset: 0 };
      continue;
    }
    if (!currentName) continue;
    const addr = line.match(/^\s*address\s*=\s*(\d+)\s*$/);
    if (addr) {
      registers[currentName].address = Number(addr[1]);
      continue;
    }
    const scale = line.match(/^\s*scale\s*=\s*([0-9.eE+-]+)\s*$/);
    if (scale) {
      registers[currentName].scale = Number(scale[1]);
      continue;
    }
    const offset = line.match(/^\s*offset\s*=\s*([0-9.eE+-]+)\s*$/);
    if (offset) {
      registers[currentName].offset = Number(offset[1]);
      continue;
    }
  }
  return registers;
}

const args = parseArgs(process.argv);
const listen = (args.listen ?? "0.0.0.0:5502").split(":");
const port = Number(listen[1]);
const listenHost = listen[0];
const registersPath = path.resolve(args.registers ?? "config/device.toml");
const faultDropPct = Number(args["drop-pct"] ?? 0);
const seedTempC = Number(args["seed-temp-c"] ?? 55);
const seedRpm = Number(args["seed-rpm"] ?? 300);
const seedPressureMpa = Number(args["seed-pressure-mpa"] ?? 0.4);
const seedShakeCpm = Number(args["seed-shake-cpm"] ?? 30);
const registerMap = loadRegisterMap(registersPath);

// Build address -> register-name map. Multiple names may share an address
// (e.g. target_temperature_c is the writable twin of temperature_c in some
// configs); we keep the first occurrence.
const addressToName = new Map();
for (const [name, info] of Object.entries(registerMap)) {
  if (info.address == null) continue;
  if (!addressToName.has(info.address)) addressToName.set(info.address, name);
}
const maxAddress = Math.max(...Array.from(addressToName.keys()), 0);
const holding = new Uint16Array(maxAddress + 32);

// Seed the writable "target" registers with realistic defaults and the
// "current" sample with a slowly varying reading.
function seed() {
  if (addressToName.has(1)) holding[1] = Math.round(seedRpm); // stirrer_rpm
  if (addressToName.has(2)) holding[2] = Math.round(seedPressureMpa * 100); // pressure_mpa scale 0.01
  if (addressToName.has(3)) holding[3] = seedShakeCpm;
  if (addressToName.has(7)) holding[7] = Math.round(7 * 100); // pH default 7
  if (addressToName.has(13)) holding[13] = Math.round(seedPressureMpa * 100); // target_pressure_mpa
  if (addressToName.has(101)) holding[101] = Math.round(seedTempC * 10); // temperature_c scale 0.1
}
seed();

let sampleTick = 0;
setInterval(() => {
  sampleTick++;
  // Drift current readings slightly so the live stream looks alive.
  if (addressToName.has(101)) {
    const drift = Math.sin(sampleTick / 4) * 0.3;
    holding[101] = Math.round((seedTempC + drift) * 10);
  }
}, 500);

function pickDrop() {
  return Math.random() * 100 < faultDropPct;
}

function buildReadResponse(functionCode, startAddress, quantity) {
  // FC 01/02/03/04: read coils / discrete inputs / holding / input registers.
  // For this mock all "registers" are 16-bit and live in the holding table.
  const byteCount = quantity * 2;
  const payload = Buffer.alloc(3 + byteCount);
  payload[0] = functionCode;
  payload[1] = byteCount;
  for (let i = 0; i < quantity; i++) {
    const addr = startAddress + i;
    const value = addr < holding.length ? holding[addr] : 0;
    payload[2 + i * 2] = (value >> 8) & 0xff;
    payload[3 + i * 2] = value & 0xff;
  }
  return payload;
}

function buildWriteSingleResponse(functionCode, address, value) {
  return Buffer.from([functionCode, (address >> 8) & 0xff, address & 0xff, (value >> 8) & 0xff, value & 0xff]);
}

function buildExceptionResponse(functionCode, exceptionCode) {
  return Buffer.from([functionCode | 0x80, exceptionCode]);
}

function handleFrame(pdu) {
  if (pdu.length < 1) return null;
  const fc = pdu[0];
  switch (fc) {
    case 0x01:
    case 0x02:
    case 0x03:
    case 0x04: {
      if (pdu.length < 5) return buildExceptionResponse(fc, 0x03); // IllegalDataValue
      const startAddress = (pdu[1] << 8) | pdu[2];
      const quantity = (pdu[3] << 8) | pdu[4];
      if (quantity < 1 || quantity > 125) return buildExceptionResponse(fc, 0x03);
      if (startAddress + quantity > holding.length) {
        return buildExceptionResponse(fc, 0x02); // IllegalDataAddress
      }
      return buildReadResponse(fc, startAddress, quantity);
    }
    case 0x05: {
      if (pdu.length < 5) return buildExceptionResponse(fc, 0x03);
      const address = (pdu[1] << 8) | pdu[2];
      const value = (pdu[3] << 8) | pdu[4];
      // Coils: 0xFF00 = ON, 0x0000 = OFF. We model coils in the high
      // address space only as a sentinel and reject below 1000.
      if (address < 1000) return buildExceptionResponse(fc, 0x02);
      return buildWriteSingleResponse(fc, address, value);
    }
    case 0x06: {
      if (pdu.length < 5) return buildExceptionResponse(fc, 0x03);
      const address = (pdu[1] << 8) | pdu[2];
      const value = (pdu[3] << 8) | pdu[4];
      if (address >= holding.length) return buildExceptionResponse(fc, 0x02);
      holding[address] = value & 0xffff;
      return buildWriteSingleResponse(fc, address, holding[address]);
    }
    case 0x0f:
    case 0x10: {
      if (fc === 0x10 && pdu.length < 6) return buildExceptionResponse(fc, 0x03);
      const startAddress = (pdu[1] << 8) | pdu[2];
      const quantity = (pdu[3] << 8) | pdu[4];
      if (quantity < 1 || quantity > 123) return buildExceptionResponse(fc, 0x03);
      // For simplicity we ignore the payload bytes; the daemon
      // never uses multi-write so this branch is for completeness.
      return Buffer.from([fc, (startAddress >> 8) & 0xff, startAddress & 0xff, (quantity >> 8) & 0xff, quantity & 0xff]);
    }
    default:
      return buildExceptionResponse(fc, 0x01); // IllegalFunction
  }
}

const server = net.createServer((client) => {
  client.on("data", (chunk) => {
    if (pickDrop()) {
      process.stdout.write(`[mock-modbus] dropped ${chunk.length} bytes\n`);
      return;
    }
    // MBAP: 7-byte header + PDU. transaction id (2), protocol id (2,
    // must be 0), length (2), unit id (1), then PDU. We ignore the
    // transaction / unit ids because the daemon and external tools all
    // pass arbitrary values.
    if (chunk.length < 8) {
      client.end();
      return;
    }
    const pdu = chunk.subarray(7);
    const response = handleFrame(pdu);
    if (response) {
      const mbap = Buffer.alloc(7);
      // Echo the request transaction id when we can read it.
      mbap[0] = chunk[0];
      mbap[1] = chunk[1];
      mbap[2] = 0; // protocol id 0 = Modbus
      mbap[3] = 0;
      mbap[4] = ((response.length + 1) >> 8) & 0xff;
      mbap[5] = (response.length + 1) & 0xff;
      mbap[6] = chunk[6] ?? 1; // unit id
      client.write(Buffer.concat([mbap, response]));
    }
  });
  client.on("error", () => {});
});

server.listen(port, listenHost, () => {
  process.stdout.write(
    `stm32-modbus-tcp-mock listening on ${listenHost}:${port} (registers=${registersPath}, drop=${faultDropPct}%)\n`
  );
});
