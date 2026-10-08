#!/usr/bin/env node
/** Verify that the public npm registry contains the exact tested tarball. */
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { parseArgs } from "node:util";

export const NOT_FOUND = 3;

class RegistryLookupError extends Error {
  constructor(message, retryable = false) {
    super(message);
    this.retryable = retryable;
  }
}

async function queryVersion(packageName, version, fetchImpl, timeoutMs) {
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), timeoutMs);
  const url = `https://registry.npmjs.org/${encodeURIComponent(packageName)}/${encodeURIComponent(version)}`;
  try {
    let response;
    try {
      response = await fetchImpl(url, {
        headers: { Accept: "application/json" },
        signal: controller.signal,
      });
    } catch (error) {
      throw new RegistryLookupError(`npm registry request failed: ${error.message}`, true);
    }
    if (response.status === 404) return null;
    if (!response.ok) {
      throw new RegistryLookupError(
        `npm registry lookup returned HTTP ${response.status}`,
        response.status === 429 || response.status >= 500,
      );
    }
    try {
      const metadata = await response.json();
      if (!metadata || typeof metadata !== "object" || Array.isArray(metadata)) {
        throw new Error("expected a package-version object");
      }
      return metadata;
    } catch (error) {
      throw new RegistryLookupError(`npm registry returned invalid JSON: ${error.message}`);
    }
  } finally {
    clearTimeout(timeout);
  }
}

/** Return 0 on an exact match, 3 only on confirmed absence; all other failures throw. */
export async function verifyPackage({
  packageName,
  version,
  tarball,
  attempts = 1,
  delayMs = 0,
  timeoutMs = 5_000,
  fetchImpl = globalThis.fetch,
  pause = (milliseconds) => new Promise((done) => setTimeout(done, milliseconds)),
  log = console.log,
}) {
  if (!packageName || !version || !tarball) throw new Error("package, version and tarball are required");
  if (!Number.isInteger(attempts) || attempts < 1) throw new Error("attempts must be a positive integer");
  if (!Number.isFinite(delayMs) || delayMs < 0) throw new Error("delay-ms must be nonnegative");
  if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) throw new Error("timeout-ms must be positive");
  const expected = `sha512-${createHash("sha512").update(readFileSync(tarball)).digest("base64")}`;
  for (let attempt = 1; attempt <= attempts; attempt++) {
    let metadata;
    try {
      metadata = await queryVersion(packageName, version, fetchImpl, timeoutMs);
    } catch (error) {
      if (!error.retryable || attempt === attempts) throw error;
      log(`${error.message}; retrying (${attempt}/${attempts})`);
      await pause(delayMs);
      continue;
    }
    if (metadata !== null) {
      if (metadata.name !== packageName || metadata.version !== version) {
        throw new Error("npm registry package identity does not match the requested version");
      }
      if (typeof metadata.dist?.integrity !== "string" || metadata.dist.integrity.trim() !== expected) {
        throw new Error(`npm ${packageName}@${version} tarball SHA-512 integrity mismatch`);
      }
      log(`npm ${packageName}@${version} matches the tested tarball`);
      return 0;
    }
    if (attempt < attempts) {
      log(`npm ${packageName}@${version} is not visible yet (${attempt}/${attempts}); retrying`);
      await pause(delayMs);
    }
  }
  log(`npm ${packageName}@${version} is not published`);
  return NOT_FOUND;
}

async function main() {
  const { values } = parseArgs({
    options: {
      package: { type: "string" },
      version: { type: "string" },
      tarball: { type: "string" },
      attempts: { type: "string", default: "1" },
      "delay-ms": { type: "string", default: "0" },
      "timeout-ms": { type: "string", default: "5000" },
    },
  });
  return verifyPackage({
    packageName: values.package,
    version: values.version,
    tarball: values.tarball,
    attempts: Number(values.attempts),
    delayMs: Number(values["delay-ms"]),
    timeoutMs: Number(values["timeout-ms"]),
  });
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  main().then((status) => { process.exitCode = status; }).catch((error) => {
    console.error(`error: ${error.message}`);
    process.exitCode = 1;
  });
}
