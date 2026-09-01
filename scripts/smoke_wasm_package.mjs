import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import initialize, { load_masterdata, recommend } from "@empty-sekai/allium-deck-wasm";

const moduleUrl = import.meta.resolve("@empty-sekai/allium-deck-wasm");
const packageRoot = path.dirname(fileURLToPath(moduleUrl));
const wasm = fs.readFileSync(path.join(packageRoot, "allium_deck_bg.wasm"));

if (typeof initialize !== "function") {
  throw new Error("WASM package default initializer export is missing");
}
if (typeof load_masterdata !== "function") {
  throw new Error("WASM package load_masterdata export is missing");
}
if (typeof recommend !== "function") {
  throw new Error("WASM package recommend export is missing");
}

await initialize({ module_or_path: wasm });

// masterdata 由调用方运行时提供；未加载时 recommend 必须报"未初始化"，而不是成功。
try {
  recommend("{}", "{}");
  throw new Error("recommend unexpectedly succeeded without masterdata");
} catch (error) {
  const message = String(error);
  if (message.includes("recommend unexpectedly succeeded")) {
    throw error;
  }
  if (!message.includes("masterdata 未初始化")) {
    throw error;
  }
}

console.log("WASM npm package import and external-masterdata initialization succeeded");
