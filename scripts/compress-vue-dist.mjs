#!/usr/bin/env node

import { brotliCompress, gzip } from "node:zlib";
import { constants } from "node:zlib";
import { promisify } from "node:util";
import { readFile, rename, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const brotliCompressAsync = promisify(brotliCompress);
const gzipAsync = promisify(gzip);
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const inputPath = path.join(root, "frontend", "dist", "index.html");
const input = await readFile(inputPath);

const [brotli, gzipped] = await Promise.all([
  brotliCompressAsync(input, {
    params: {
      [constants.BROTLI_PARAM_QUALITY]: 11,
      [constants.BROTLI_PARAM_MODE]: constants.BROTLI_MODE_TEXT,
    },
  }),
  gzipAsync(input, { level: 9 }),
]);

await Promise.all([
  writeAtomically(`${inputPath}.br`, brotli),
  writeAtomically(`${inputPath}.gz`, gzipped),
]);

console.log(
  JSON.stringify(
    {
      input: path.relative(root, inputPath),
      bytes: input.length,
      brotliBytes: brotli.length,
      gzipBytes: gzipped.length,
      brotliRatio: Number((brotli.length / input.length).toFixed(4)),
      gzipRatio: Number((gzipped.length / input.length).toFixed(4)),
    },
    null,
    2,
  ),
);

async function writeAtomically(outputPath, data) {
  const temporaryPath = `${outputPath}.${process.pid}.tmp`;
  await writeFile(temporaryPath, data);
  await rename(temporaryPath, outputPath);
}
