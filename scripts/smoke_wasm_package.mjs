import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import initialize, { recommend_embedded } from "@empty-sekai/allium-deck-wasm";

const moduleUrl = import.meta.resolve("@empty-sekai/allium-deck-wasm");
const packageRoot = path.dirname(fileURLToPath(moduleUrl));
const wasm = fs.readFileSync(path.join(packageRoot, "allium_deck_bg.wasm"));

if (typeof initialize !== "function") {
  throw new Error("WASM package default initializer export is missing");
}
if (typeof recommend_embedded !== "function") {
  throw new Error("WASM package recommend_embedded export is missing");
}

await initialize({ module_or_path: wasm });

try {
  recommend_embedded("{}", "{}");
  throw new Error("recommend_embedded unexpectedly succeeded");
} catch (error) {
  const message = String(error);
  if (message.includes("recommend_embedded unexpectedly succeeded")) {
    throw error;
  }
  if (message !== "候选卡池为空") {
    throw error;
  }
}

console.log("WASM npm package import and embedded-data initialization succeeded");
