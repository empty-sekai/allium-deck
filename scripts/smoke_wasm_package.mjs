import fs from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";

const installRoot = process.argv[2];
if (!installRoot) {
  throw new Error("usage: node scripts/smoke_wasm_package.mjs <npm-install-root>");
}

const packageRoot = path.join(
  path.resolve(installRoot),
  "node_modules",
  "@empty-sekai",
  "allium-deck-wasm",
);
const moduleUrl = pathToFileURL(path.join(packageRoot, "allium_deck.js"));
const deck = await import(moduleUrl.href);
const wasm = fs.readFileSync(path.join(packageRoot, "allium_deck_bg.wasm"));

await deck.default({ module_or_path: wasm });

try {
  deck.recommend_embedded("{}", "{}");
} catch (error) {
  const message = String(error);
  if (message.includes("postcard") || message.includes("内嵌 masterdata")) {
    throw new Error(`embedded masterdata failed to load: ${message}`);
  }
}

console.log("WASM npm package import and embedded-data initialization succeeded");
