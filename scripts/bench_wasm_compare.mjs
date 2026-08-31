// 端到端对拍基准（Node）：本引擎 wasm vs 参考引擎 wasm。
//
// 计时口径：
// - 每次 timed call 都是完整消费链路：字符串入 → 用户/参数解析 → 组池 → 搜索 → 字符串出；
//   参考引擎侧 createUserData 同样计入计时。
// - masterdata 载入按真实消费形态计时：本引擎收 JSON 文本（含外层 stringify），
//   参考引擎收对象表（内部逐表 stringify 后解析）——两边都要付 stringify+parse。
// - 终章场景一律走 event_id=180 直连（真实 masterdata 数据）；
//   WL3 模拟终章（3_200_000）仅由本引擎提供，无对拍 oracle，只做单测锁定。
// - 覆盖：马拉松(multi)、烤森(cheerful)、WL1/3 回合、终章(180)、JP 区服，
//   以及区域道具/曲目推荐/精确打歌分/WL 支援卡四个辅助接口。
//
// 运行前一次性环境准备（.tmp/bench/）：
//   git clone --depth 1 https://github.com/Team-Haruki/haruki-sekai-sc-master cn-repo
//   git clone --depth 1 https://github.com/Team-Haruki/haruki-sekai-master jp-repo
//   # 把两仓 master/*.json 拷到 masterdata/{cn,jp}/（含辅助表
//   # areas/areaItems/shopItems/ingameNotes/ingameCombos）
//   curl music_metas（cn: cdn.emptysekai.com/music_metas/cn/latest；jp: sekai-data.3-3.dev）
//   cd cpp-pkg && npm install haruki-sekai-deck-recommend-cpp
//   cp wasm/pkg/{allium_deck.js,allium_deck_bg.wasm} ours-pkg/
// 运行：node scripts/bench_wasm_compare.mjs
import { readFileSync, readdirSync } from "node:fs";
import { createRequire } from "node:module";
import { pathToFileURL } from "node:url";
import { join } from "node:path";
import { gzipSync } from "node:zlib";

const BENCH = "F:/allium/.tmp/bench";
const OURS = join(BENCH, "ours-pkg");
const CPP_DIR = join(BENCH, "cpp-pkg/node_modules/haruki-sekai-deck-recommend-cpp");
const require = createRequire(import.meta.url);

const RUNS = 5;           // 每场景计时轮数（取中位数）
const AUX_RUNS = 3;       // 辅助接口计时轮数
const SCORE_REL_TOL = 1e-4;

const median = (values) => {
    const sorted = [...values].sort((a, b) => a - b);
    return sorted[Math.floor(sorted.length / 2)];
};
const rel = (a, b) => (b === 0 ? (a === 0 ? 0 : Infinity) : Math.abs(a - b) / Math.abs(b));

// ---------- 两引擎 ----------
const ours = await import(pathToFileURL(join(OURS, "allium_deck.js")).href);
await ours.default({ module_or_path: readFileSync(join(OURS, "allium_deck_bg.wasm")) });
const cppMod = await import(pathToFileURL(join(CPP_DIR, "index.js")).href);
const cppWasm = readFileSync(join(CPP_DIR, "sekai_deck_recommend.wasm"));
// 参考引擎的高层封装（createSekaiDeckRecommend → SekaiDeckRecommendWasm）：
// options/handle 为对象、内部 stringify——与本引擎字符串入参对称。
const cpp = await cppMod.createSekaiDeckRecommend({
    moduleOptions: {
        wasmBinary: cppWasm,
        print: () => {},
        printErr: () => {},
    },
});

// ---------- 消费端真实输入 ----------
const userPayload = JSON.parse(readFileSync(
    "F:/allium/allium-deck-oss/testdata/real/_test_world_bloom_input.json", "utf-8"));
const userCn = userPayload.user_data_str;
const userJp = JSON.stringify({ ...JSON.parse(userCn), userHonors: [] });

// ---------- masterdata 文本表（本引擎）与对象表（参考引擎）----------
const textMap = { cn: {}, jp: {} };
const objMap = { cn: {}, jp: {} };
for (const region of ["cn", "jp"]) {
    for (const f of readdirSync(join(BENCH, "masterdata", region))) {
        if (!f.endsWith(".json")) continue;
        const text = readFileSync(join(BENCH, "masterdata", region, f), "utf-8");
        textMap[region][f] = text;
        try { objMap[region][f.slice(0, -5)] = JSON.parse(text); } catch { /* 跳过 */ }
    }
}
const metas = {
    cn: readFileSync(join(BENCH, "music_metas-cn.json"), "utf-8"),
    jp: readFileSync(join(BENCH, "music_metas-jp.json"), "utf-8"),
};

// 从 events 表取某类型最新活动 id（马拉松/烤森场景用）
const latestEvent = (region, type) => {
    const events = objMap[region].events.filter((e) => e.eventType === type);
    if (!events.length) throw new Error(`no ${type} event in ${region}`);
    return Math.max(...events.map((e) => e.id));
};

// ---------- 冷启动：masterdata 载入（全链路：stringify + parse + flatten）----------
async function benchLoad(region) {
    const oursMs = [], cppMs = [];
    for (let i = 0; i < 3; i++) {
        // 交错执行，避免 JIT/缓存偏向某一方
        let t0 = performance.now();
        cpp.loadMasterData(region, objMap[region]);
        cpp.loadMusicMetas(region, metas[region]);
        cppMs.push(performance.now() - t0);
        t0 = performance.now();
        ours.load_masterdata(JSON.stringify(textMap[region]), metas[region]);
        oursMs.push(performance.now() - t0);
    }
    return { ours: median(oursMs), cpp: median(cppMs) };
}

// ---------- recommend E2E ----------
const regions = {
    cn: {
        user: userCn,
        scenarios: [
            { name: "cn_marathon_multi", live_type: "multi", event_id: latestEvent("cn", "marathon") },
            { name: "cn_cheerful_multi", live_type: "cheerful", event_id: latestEvent("cn", "cheerful_carnival") },
            { name: "cn_wl1_turn1", live_type: "solo", event_id: 112, event_type: "world_bloom", world_bloom_character_id: 18, world_bloom_event_turn: 1 },
            { name: "cn_wl2_finale", live_type: "solo", event_id: 180, event_type: "world_bloom" },
        ],
    },
    jp: {
        user: userJp,
        scenarios: [
            { name: "jp_wl2_finale", live_type: "solo", event_id: 180, event_type: "world_bloom" },
            { name: "jp_wl3_turn_sim", live_type: "solo", event_type: "world_bloom", world_bloom_event_turn: 3, world_bloom_character_id: 1 },
        ],
    },
};

const baseParams = {
    region: "", live_type: "solo", music_id: 1, music_diff: "master",
    target: "score", algorithm: "dfs", limit: 5, timeout_ms: 30000,
};

function ourRecommend(user, params) {
    return JSON.parse(ours.recommend(user, JSON.stringify(params)));
}
function cppRecommend(region, user, params) {
    const handle = cpp.createUserData(region, JSON.parse(user));
    return cpp.recommend(params, handle);
}

function verifyDecks(name, ourOut, cppOut) {
    const ourDeck = ourOut.decks?.[0];
    const cppDeck = cppOut.decks?.[0];
    if (!ourDeck || !cppDeck) return `SKIP 无卡组`;
    const ourScore = ourDeck.event_point ?? ourDeck.live_score;
    const cppScore = cppDeck.score ?? cppDeck.event_point;
    const r = rel(ourScore, cppScore);
    if (r > SCORE_REL_TOL) return `FAIL score ours=${ourScore} cpp=${cppScore}`;
    // 分数一致即最优；并列最优下双方报告的卡组可以不同（事件点整数化产生大量并列）。
    const ourCards = ourDeck.cards.map((c) => c.card_id).sort((a, b) => a - b).join(",");
    const cppCards = cppDeck.cards.map((c) => c.card_id).sort((a, b) => a - b).join(",");
    return ourCards === cppCards ? `PASS score=${cppScore}` : `PASS score=${cppScore}（并列最优，卡组不同）`;
}

async function benchRecommend(region, scenario) {
    const user = regions[region].user;
    const params = { ...baseParams, region, ...scenario };
    delete params.name;
    // 预热（JIT + 缓存）
    ourRecommend(user, params);
    cppRecommend(region, user, params);
    const oursMs = [], cppMs = [];
    let verdict = "PASS";
    for (let i = 0; i < RUNS; i++) {
        let t0 = performance.now();
        const cppOut = cppRecommend(region, user, params);
        cppMs.push(performance.now() - t0);
        t0 = performance.now();
        const ourOut = ourRecommend(user, params);
        oursMs.push(performance.now() - t0);
        verdict = verifyDecks(scenario.name, ourOut, cppOut);
    }
    return { ours: median(oursMs), cpp: median(cppMs), verdict };
}

// ---------- 辅助接口 E2E ----------
function topDeckOf(region, scenario) {
    const user = regions[region].user;
    const params = { ...baseParams, region, ...scenario };
    delete params.name;
    return ourRecommend(user, params).decks[0];
}

function deckJsonFrom(deck) {
    return {
        total_power: deck.total_power,
        event_bonus_rate: deck.event_bonus_total ?? 0,
        support_deck_bonus_rate: 0,
        cards: deck.cards.map((c) => ({
            skill_score_up: c.skill_score_up,
            skill_life_recovery: 0,
        })),
    };
}

function verifyRows(a, b, keyOf) {
    if (!Array.isArray(a) || !Array.isArray(b)) return "FAIL 非数组输出";
    if (a.length !== b.length) return `FAIL 行数 ours=${a.length} cpp=${b.length}`;
    for (let i = 0; i < b.length; i++) {
        for (const [key, tol] of Object.entries(keyOf)) {
            const va = a[i][key], vb = b[i][key];
            const diff = tol === 0 ? (va === vb ? 0 : Infinity) : rel(va, vb);
            if (!(diff <= tol)) return `FAIL 第${i}行 ${key} ours=${va} cpp=${vb}`;
        }
    }
    return `PASS x${b.length}`;
}

async function benchAux(region, label, runOurs, runCpp, verify) {
    runOurs(); runCpp(); // 预热
    const oursMs = [], cppMs = [];
    let verdict = "PASS";
    for (let i = 0; i < AUX_RUNS; i++) {
        let t0 = performance.now();
        const cppOut = runCpp();
        cppMs.push(performance.now() - t0);
        t0 = performance.now();
        const ourOut = runOurs();
        oursMs.push(performance.now() - t0);
        verdict = verify(ourOut, cppOut);
    }
    return { label, ours: median(oursMs), cpp: median(cppMs), verdict };
}

async function benchAuxSuite(region) {
    const user = regions[region].user;
    const results = [];

    // 精确打歌分：同一谱面文本，逐字段比对（同公式，期望位级一致）
    const musicScore = JSON.stringify({
        notes: Array.from({ length: 600 }, (_, i) => ({ time: i * 0.4, type: (i % 3) + 1 })),
        skills: Array.from({ length: 6 }, (_, i) => ({ time: 10 + i * 30 })),
        fevers: [{ time: 120 }],
    });
    const exactOpts = JSON.stringify({
        region, live_type: "multi", power: 200000, skills: [100, 200, 300, 400, 500],
        music_score: musicScore, fever_music_score: musicScore, multi_sum_power: 1000000,
    });
    results.push(await benchAux(region, "calculateExactLive",
        () => JSON.parse(ours.calculate_exact_live(exactOpts)),
        () => cpp.calculateExactLive(JSON.parse(exactOpts)),
        (a, b) => {
            const r = rel(a.total, b.total);
            return r <= 1e-9 ? `PASS total=${b.total.toFixed(1)} notes=${b.notes.length}`
                : `FAIL total ours=${a.total} cpp=${b.total} rel=${r}`;
        }));

    // 组卡一次，产出卡组供区域道具/曲目推荐使用（真实消费链路）
    const deck = topDeckOf(region, { name: "x", live_type: "multi", event_id: latestEvent(region, "marathon") });
    const cardIds = deck.cards.map((c) => c.card_id);

    // 区域道具推荐：按行对齐，比对 power/cost/shop_item_id
    const areaOpts = JSON.stringify({ region, user_data: JSON.parse(user), card_ids: cardIds });
    results.push(await benchAux(region, "recommendAreaItems",
        () => JSON.parse(ours.recommend_area_items(areaOpts)),
        () => cpp.recommendAreaItems(JSON.parse(areaOpts)),
        (a, b) => verifyRows(a, b, { area_item_id: 0, next_level: 0, power: 1e-6, shop_item_id: 0 })));

    // 曲目推荐：同一卡组 JSON，比对全部行的 live_score / event_point
    const musicOpts = JSON.stringify({
        region, live_type: "multi", event_id: latestEvent(region, "marathon"),
        deck: deckJsonFrom(deck),
    });
    const musicOptsObj = JSON.parse(musicOpts);
    results.push(await benchAux(region, "recommendMusic",
        () => JSON.parse(ours.recommendMusic(musicOpts)),
        () => cpp.recommendMusic(musicOptsObj, musicOptsObj.deck),
        (a, b) => {
            // 本引擎会额外合成 omakase 曲目行，因此要求参考引擎行集 ⊆ 本引擎且逐行一致。
            const key = (r) => `${r.music_id}|${r.difficulty}`;
            const oursMap = new Map(a.map((r) => [key(r), r]));
            let bad = 0;
            for (const row of b) {
                const oursRow = oursMap.get(key(row));
                if (!oursRow || oursRow.live_score !== row.live_score
                    || oursRow.event_point !== row.event_point) bad++;
            }
            const extra = a.length - b.length;
            return bad === 0
                ? `PASS cpp ${b.length} 行全一致（本引擎另有 omakase +${extra} 行）`
                : `FAIL ${bad}/${b.length} 行不一致`;
        }));

    // WL 支援卡（jp 有完整 WL3 数据走模拟终章；cn 走 legacy 终章）
    const wbOpts = JSON.stringify(region === "jp"
        ? { region, user_data_str: user, world_bloom_finale_turn: 3, world_bloom_character_id: 1 }
        : { region, user_data_str: user, world_bloom_finale_turn: 2, world_bloom_character_id: 1 });
    results.push(await benchAux(region, "getWBSupportCards",
        () => JSON.parse(ours.get_world_bloom_support_cards(wbOpts)),
        () => cpp.getWorldBloomSupportCards(JSON.parse(wbOpts)),
        (a, b) => {
            const n = Math.min(10, b.length);
            for (let i = 0; i < n; i++) {
                if (a[i].card_id !== b[i].card_id || rel(a[i].bonus, b[i].bonus) > 1e-6) {
                    return `FAIL 第${i}名 ours=${a[i].card_id}/${a[i].bonus} cpp=${b[i].card_id}/${b[i].bonus}`;
                }
            }
            return `PASS top${n}`;
        }));

    return results;
}

// ---------- 主流程 ----------
const sizeOf = (p) => readFileSync(p).length;
const gzOf = (p) => gzipSync(readFileSync(p), { level: 9 }).length;
console.log(`体积 raw：ours=${sizeOf(join(OURS, "allium_deck_bg.wasm"))}B cpp=${sizeOf(join(CPP_DIR, "sekai_deck_recommend.wasm"))}B`);
console.log(`体积 gzip9：ours=${gzOf(join(OURS, "allium_deck_bg.wasm"))}B cpp=${gzOf(join(CPP_DIR, "sekai_deck_recommend.wasm"))}B`);

const report = [];
for (const region of ["cn", "jp"]) {
    const load = await benchLoad(region);
    report.push(`[冷启动 masterdata 载入 ${region.toUpperCase()}] ours=${load.ours.toFixed(0)}ms cpp=${load.cpp.toFixed(0)}ms ratio=${(load.ours / load.cpp).toFixed(2)}x`);
    for (const scenario of regions[region].scenarios) {
        const r = await benchRecommend(region, scenario);
        report.push(`[recommend ${scenario.name}] ours=${r.ours.toFixed(1)}ms cpp=${r.cpp.toFixed(1)}ms ratio=${(r.ours / r.cpp).toFixed(2)}x ${r.verdict}`);
    }
    for (const aux of await benchAuxSuite(region)) {
        report.push(`[辅助 ${aux.label}] ours=${aux.ours.toFixed(1)}ms cpp=${aux.cpp.toFixed(1)}ms ratio=${(aux.ours / aux.cpp).toFixed(2)}x ${aux.verdict}`);
    }
}
console.log("\n===== 端到端对拍结果 =====");
for (const line of report) console.log(line);
const fails = report.filter((l) => l.includes("FAIL"));
console.log(fails.length ? `\n${fails.length} 项 FAIL` : "\n全部 PASS");
process.exit(fails.length ? 1 : 0);
