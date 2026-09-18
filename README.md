# allium-deck

[English](./README.en.md) | 简体中文

Project Sekai 组卡推荐引擎的 Rust 实现，专攻 **DFS / 分支限界（B&B）精确搜索**。

给定玩家卡组、活动加成与目标（综合力 / 技能 / 活动点数 / MySekai 等），在巨大的组合空间里搜出最优的 5 张卡编成。核心数据结构用 SoA（结构体数组）+ 位运算组织，配合角色感知的后缀上界与支配剪枝；常规 260-card Top-1 / Top-8 热路径已经进入亚毫秒级，更重的 Top-K 与对抗输入见下方实测。

## 关于实现来源

部分游戏内数值与逻辑（综合力、技能加成、活动点数、支援卡组、WL3 模拟终章等）来自下列开源实现，源码注释保留对照出处：

- https://github.com/Team-Haruki/sekai-deck-recommend-cpp
- https://github.com/StarMoe-org/sekai-deck-recommend-cpp

具体移植与修正内容见各 commit 说明。

在此基础上，本实现并非逐行翻译，而是对**底层热路径与搜索剪枝做了彻底的 Rust 重构**，核心数据结构全部按 cache line 对齐：

- **建池（pool building）**：对 masterdata 一次性建立 by-id 索引，把逐卡 O(N) 的线性扫描降为 O(1) 查表。
- **搜索（search）**：SoA 卡池 + 512-bit 候选位图、综合力压成 u18×8 槽位 + 查找表、逐角色支配裁剪、角色感知后缀上界、贪心 + 1-swap warm start 下界，使分支限界尽早剪枝。

`CardPool` 采用列式 SoA 布局，每列 64 字节对齐。典型候选池（130–260 张卡）整体 **~7–12 KB**，加上 `SearchContext`、`SuffixBound` 等搜索期结构，热路径数据适合驻留在现代服务器 CPU 的 L1 data cache 内（EPYC 9K85 每核 L1d = 48 KiB）。叶子评估遍历卡组时按列顺序访问，尽量减少无关 cache line 和 TLB 压力。

## 性能

下面是当前最终代码在 **AMD EPYC 9K85** 上的原生 release 实测。进程固定在 CPU 2，并限制为 80% CPU；每个场景都使用 260 张合成候选卡。`建池` 是从已经解析好的玩家数据构造 `CardPool` / 搜索上下文的时间；`搜索` 包含支配裁剪、上界构造、warm start、主搜索与 Top-K 替代恢复，不包含 fixture 文件读取和 JSON 解析。

| 场景 | Top-K | 建池 | 搜索 p50 | 搜索 p95 |
| --- | ---: | ---: | ---: | ---: |
| balanced / Solo / 无活动 | 1 | 0.390 ms | **0.632 ms** | 0.656 ms |
| balanced / Solo / 无活动 | 8 | 0.359 ms | **0.747 ms** | 0.767 ms |
| balanced / Multi / 无活动 | 8 | 0.371 ms | **0.365 ms** | 0.401 ms |
| balanced / Solo / 活动 | 8 | 0.382 ms | **1.468 ms** | 2.057 ms |
| tradeoff / Solo / 无活动 | 1 | 0.338 ms | **0.994 ms** | 1.012 ms |
| tradeoff / Solo / 无活动 | 8 | 0.340 ms | **4.378 ms** | 21.329 ms |
| tradeoff / Solo / 无活动 | 100 | 0.345 ms | **13.467 ms** | 34.460 ms |

`balanced260` 用来代表常规高练度卡池；`tradeoff260` 刻意让同角色的 power / skill 强负相关，是用于放大搜索长尾的压力输入。实际耗时会随账号规模、活动规则、目标和 Top-K 改变，所以这里同时给 p50 / p95，而不是只挑最快的一次。

最终候选还做了更重的分布级 A/B：普通压力集 Top-1 / 8 / 30 / 100 的 p50 相比上一条精确基线分别降低约 **20.2% / 14.8% / 16.7% / 17.7%**；Final-auto 分别降低约 **20.6% / 17.0% / 15.2% / 7.7%**。所有双方都完整结束的对应样本结果完全一致，2 秒压力超时的数量也没有增加。

x86-64 会在运行时检测 AVX-512F/BW；不支持的 CPU 和其他架构自动使用 scalar fallback。

## 对外 API

主入口是 `engine::recommend_json`——纯 JSON 进、JSON 出：

```rust
use allium_deck::engine::recommend_json;

let result_json = recommend_json(
    masterdata_json,   // 游戏 masterdata
    music_metas_json,  // 歌曲元数据
    user_data_json,    // 玩家卡组（camelCase）
    params_json,       // 组卡参数（target / event / card_configs 等）
)?;
```

内部走两阶段：`handler::build_card_pool`（建池）→ `search::search`（搜索）。结构体入口 `engine::recommend` 可绕过 JSON 序列化。

返回 `{"decks": [...], "completion": "complete", "stats": {...}}`。`cards` 是游戏卡 ID，按站位顺序、队长在前；`score` 是搜索排序键。综合力、live 分数等面板明细不在其中，需要时用 `handler::build_card_pool` + `search::summarize_deck`（`src/bin/recommend_cli.rs` 是完整示例）。

完整参数与模式说明见 [docs/parameters.md](docs/parameters.md)。搜索完整结束时，各受支持模式都返回精确 Top-K；超时会显式返回 `timed_out`，不会伪装成完整结果。整体正确性说明见 [docs/exactness-proof.md](docs/exactness-proof.md)，每一种剪枝为什么不会漏解的形式化证明见 [docs/pruning-proof.md](docs/pruning-proof.md)。

## 模块地图

| 模块 | 职责 |
| --- | --- |
| `engine` | 对外入口（`recommend_json` / `recommend`）、masterdata 加载（`OwnedGameData`）、JSON 参数解析 |
| `types` | 公共标识符与枚举（`Unit` / `Attr` / `LiveType` / `ScoreTarget`），以及逐卡解析后的综合力与技能数值 |
| `handler` | 建池层：候选裁剪、综合力/技能/活动加成预计算、WL 支援卡组、构建搜索上下文 |
| `pool` | SoA 卡池：列式存储、位图、对齐布局、冻结后只读 |
| `search` | 搜索层：支配剪枝、后缀上界、warm start、按目标/场景分派 B&B / DP / 专用求解器、叶子精确评估 |
| `auxiliary` | 非搜索路径的辅助计算：区域道具推荐、曲目推荐、精确打歌分（`wasm` 直接导出） |

## 数据流

```
masterdata JSON ─┐
                 ├─→ OwnedGameData::load        // 一次性加载，可缓存
music_metas ─────┘        │ as_ref()
                          ▼
                    GameData (只读借用视图)
user JSON ──→ parse_user_profile_json ──→ UserProfile
params JSON ─→ parse_build_params_json ──→ BuildParams
                          │
                          ▼
        build_card_pool (handler)              // 建池
          ├─ 每张用户卡：综合力 / 技能 / 活动加成预计算
          ├─ 硬约束过滤与安全预处理（不做质量前缀截断）
          ├─ 排序灌入 SoA CardPool
          └─ 构建 SearchContext（含 WL 支援卡组）
                          │
                          ▼  (CardPool, SearchContext)
        search (search)                        // 搜索
          ├─ 逐角色支配裁剪
          ├─ 角色感知后缀上界（B&B 剪枝核心）
          ├─ warm start 下界（贪心 + 1-swap）
          └─ 按 target / 场景分派：
               Score / MySekai B&B、Power DP / B&B、Skill B&B、Challenge、终章
                          │
                          ▼
                  Vec<DeckResult> → JSON
```

## 依赖与构建

原生核心只依赖 `serde` / `serde_json` / `thiserror`；wasm32 额外使用 `web-time` 提供单调时钟。没有图形、异步或系统库依赖，可独立编译：

```bash
cargo build --release
```

性能数字应在 release profile 下测量。`src/bin/recommend_cli.rs` 提供 standalone CLI（`cargo install allium-deck` 出来后命令名 `recommend_cli`），打印分阶段耗时（建池 vs 搜索），方便快速迭代验证。

## 语言绑定

| 语言 | 位置 | 说明 |
| --- | --- | --- |
| Rust | 本仓库（crates.io `allium-deck`） | 引擎本体 |
| JavaScript / 浏览器 | [`wasm/`](wasm)（npm `@empty-sekai/allium-deck-wasm`） | WASM 绑定，见下方导出表 |
| Python | [`allium-deck-python`](https://github.com/empty-sekai/allium-deck-python)（PyPI `allium-sekai-deck`） | 预编译 abi3 wheel，含 `allium_deck` API 与 LunaBot 兼容门面，无需本地 Rust 工具链 |

### WASM 接口面

外置 masterdata 模式（推荐，浏览器侧复用已有的 masterdata JSON）：

| 导出 | 说明 |
| --- | --- |
| `load_masterdata(map, metas)` | 一次扁平化并缓存；辅助表（areas/areaItems/shopItems/ingameNotes/ingameCombos）可选 |
| `recommend(user, params)` | 字符串入 / JSON 字符串出 |
| `createUserData(user, region)` + `recommendWithUserData(options, handle)` | 解析一次用户数据多次复用（region 词表 jp/tw/en/kr/cn） |
| `recommend_area_items(options)` | 固定卡组的区域道具升级建议 |
| `recommendMusic(options)` | 已定卡组的全曲目/难度打分排序 |
| `calculate_exact_live(options)` | 逐 note 精确打歌分 |
| `get_world_bloom_support_cards(options)` | WL 支援卡逐卡加成（按 bonus 降序、card_id 升序） |

`recommendBatch` 系列暂未提供。options 键名为 snake_case（兼容 camelCase 别名），
输出键名为 snake_case。`recommend_embedded` 仍保留在 `embedded` feature 下。

## CLI

`recommend_cli` 是可独立运行的组卡推荐命令行工具，可从 [GitHub Releases](https://github.com/empty-sekai/allium-deck/releases) 下载预编译二进制，或从源码安装。

从命令行跑一次完整推荐，打印建池/搜索耗时和 Top-K 卡组：

```bash
# 方式1: 下载预编译二进制 (以 linux-x86_64 为例)
curl -L -o recommend_cli \
  https://github.com/empty-sekai/allium-deck/releases/download/v0.0.15/recommend_cli-v0.0.15-linux-x86_64
chmod +x recommend_cli
./recommend_cli [OPTIONS]

# 方式2: 从 git 安装 (无需 clone)
cargo install --git https://github.com/empty-sekai/allium-deck --bin recommend_cli
recommend_cli [OPTIONS]

# 方式3: Clone 后本地编译
git clone https://github.com/empty-sekai/allium-deck.git
cd allium-deck
cargo build --release --bin recommend_cli
./target/release/recommend_cli [OPTIONS]
```

**使用方法：**

```bash
recommend_cli \
  --masterdata <masterdata-dir> \
  --music-metas <music_metas.json> \
  --user <user.json> \
  --target score \
  --live-type multi \
  --event-id 170 \
  --music-id 74 \
  --music-diff expert \
  --boost 10 \
  --event-unit ln \
  --event-attr cool \
  --unit-filter ln \
  --multi-teammate-power 250000 \
  --multi-teammate-score-up 200 \
  --top-k 5
```

参数：

| 参数 | 类型 | 说明 |
| --- | --- | --- |
| `--masterdata` | 目录 | 游戏 masterdata 目录，内含 `cards.json`、`events.json`、`skills.json`、`cardRarities.json`、`gameCharacterUnits.json` 等文件。 |
| `--music-metas` | 文件 | 歌曲元数据 JSON 文件。 |
| `--user` | 文件 | 玩家数据 JSON，至少包含 `userCards`；区域道具、角色等级、称号、MySekai 等字段会参与评分。 |
| `--params` | 文件 | 兼容入口：读取推荐参数 JSON；直接 flags 会覆盖同名 JSON 字段。 |
| `--target` | 枚举 | `score` / `power` / `skill` / `mysekai`。 |
| `--live-type` | 枚举 | `solo` / `multi` / `cheerful` / `auto` / `challenge` / `challenge_auto` / `mysekai`。 |
| `--event-id` / `--music-id` / `--music-diff` | 值 | 活动、歌曲和难度；难度为 `easy` / `normal` / `hard` / `expert` / `master` / `append`。 |
| `--boost` | 整数 | 火数 `0..10`：`0` 为无火，`1..5` 为 `5/10/15/20/25x`，`6..10` 为 `27/29/31/33/35x`。 |
| `--fixed-cards` / `--fixed-characters` / `--excluded-cards` | 列表 | 逗号分隔的卡 ID / 角色 ID 约束。 |
| `--event-unit` / `--event-attr` | 枚举 | 模拟活动团和属性；团可用 `ln/mmj/vbs/wxs/25ji/vs`，属性可用 `cool/cute/happy/pure/mysterious`。 |
| `--unit-filter` / `--attr-filter` | 枚举 | 硬过滤候选池；VS 双团卡按 `support_unit` 参与对应团过滤。 |
| `--world-bloom-character-id` / `--world-bloom-event-turn` / `--challenge-live-character-id` | 值 | WL / Challenge Live 特殊参数。 |
| `--mode area-items` / `--mode music` / `--mode exact-live` | 模式 | 辅助计算（不组卡）：`area-items` 需 `--card-ids`；`music` 需 `--deck`；`exact-live` 需 `--power/--skills/--music-score`。 |
| `--skill-reference-strategy` / `--live-skill-order` / `--specific-skill-order` | 值 | 技能参考与发动顺序；指定顺序使用 `0,1,2,3,4`。 |
| `--multi-teammate-power` / `--multi-teammate-score-up` / `--multi-live-score-up-lower-bound` | 值 | 协力和 Cheerful 队友综合力、技能实效、技能总下限。 |
| `--other-score` / `--life` | 值 | Cheerful 对手分数和体力。 |
| `--rarity4-config` / `--single-card-config` | 值 | 养成配置，如 `level_max,skill_max,master_max,episode_read,canvas` 和 `123:level_max,skill_max`。 |

输出示例：

`stderr` 只输出进度和耗时，`stdout` 固定输出结构化 JSON，便于回归和性能对比：

```text
[load] masterdata+music_metas: 135.0ms
[build_pool] 1.4ms  pool=78 effective_live=Multi
[search] 0.4ms  leaf=84 ub_prunes=278 ep_explored=18 mono_break=12
[total] 136.8ms
```

```json
{
  "completion": "complete",
  "timed_out": false,
  "effective_params": { "target": "Score", "live_type": "Multi", "boost": 10 },
  "diagnostics": { "pool_size": 78, "effective_live_type": "Multi" },
  "timing": { "build_pool_ms": 1.4, "search_ms": 0.4 },
  "decks": [
    {
      "rank": 1,
      "event_point": 1234567,
      "cards": [
        { "card_id": 111, "power_total": 35210, "skill_score_up": 120.0, "has_canvas_bonus": true, "canvas_power": 600 }
      ]
    }
  ]
}
```

## HTTP 服务

`server/` 是引擎的 HTTP 服务，独立 crate，不发布到 crates.io。masterdata 常驻内存，
搜索跑在固定数量的专用线程上，队列有界——满了直接回 503 而不是把请求堆进无界积压。

这是一个直接的实现：它让引擎可以通过 HTTP 用起来，并带上共享服务需要的那几道护栏，
但**不是调优过的架构，也不是完整的部署方案**。没有 TLS、没有鉴权与配额、没有缓存、
没有请求取消、不跨实例协调，单次搜索也不并行。完整的边界清单见
[`server/README.md`](./server/README.md#范围)。

```bash
# 本仓不携带游戏数据；先导出一份合成 masterdata 就能把服务跑起来
cargo run --manifest-path server/Cargo.toml --release --bin export-synth-masterdata -- ./synth

cd server
cargo run --release -- \
  --masterdata synth=../synth/masterdata \
  --music-metas synth=../synth/music_metas.json
```

```bash
curl localhost:8080/v1/recommend -H 'content-type: application/json' -d "{
  \"user\": $(cat ../synth/user.json),
  \"params\": {\"liveType\": \"multi\", \"target\": \"score\", \"eventId\": 1, \"limit\": 5}
}"
```

`params` 就是引擎自己的参数契约（见 `docs/parameters.md`），服务不另造一套方言。

| 方法 | 路径 | 用途 |
| --- | --- | --- |
| POST | `/v1/recommend` | 组卡；覆盖全部 target 与 live type，含 WL 章节与终章 |
| POST | `/v1/recommend/challenge-all` | 26 个角色各自的最优挑战卡组，带排名 |
| POST | `/v1/world-bloom/support-cards` | WL 章节的逐卡支援加成 |
| POST | `/v1/music/recommend` | 对已定卡组给全部曲目/难度打分排序 |
| POST | `/v1/live/exact-score` | 按谱面逐 note 计算打歌分 |
| POST | `/v1/area-items/recommend` | 区域道具升级的性价比排序 |
| GET | `/v1/regions` `/healthz` `/readyz` `/metrics` `/openapi.json` | 区服清单与运维端点 |

每个响应的 `timing` 会给出 queue / 建池 / 搜索的分段耗时，`/metrics` 里是同一组分段的
直方图——决定 `--workers` 和 `--max-queue` 该设多少时看这个。

```bash
# 构建上下文是仓库根目录（服务按 path 依赖引擎 crate）
docker build -f server/Dockerfile -t allium-deck-server .
docker run --rm -p 8080:8080 -v /path/to/data:/data:ro allium-deck-server   --masterdata cn=/data/masterdata --music-metas cn=/data/music_metas.json
```

完整配置项、背压与超时语义、错误码见 [`server/README.md`](./server/README.md)；
镜像的 libc / 分配器 / 运行时基底组合与实测见 [`docker/README.md`](./docker/README.md)。

## 静态数据

`data/` 内嵌 3 张世界开花（World Bloom）支援卡组加成表。这些表在参考实现中作为仓库静态资源随包携带、不随 masterdata 更新，因此这里用 `include_str!` 内嵌，masterdata 缺失对应文件时回退使用。

## 精确性

这里的“精确”指的是：**只要搜索以 `Complete` 结束，返回的就是完整可行集合按统一排序规则得到的真正 Top-K**，不是依赖随机种子或经验阈值的近似答案。Score、活动分、MySekai、World Bloom、终章、Challenge、Power、Skill 和精确加成档位都遵守这条规则。

实现里仍然有 warm start、beam、one-swap 等启发式，但它们只用于更早找到好解、提高分支限界阈值或调整访问顺序；不会拿来删除尚未被数学上界否定的候选。无约束 Power 使用精确的 49-scenario DP，其余 Power / Skill 走完整候选集上的有界搜索。

搜索有显式 deadline。命中 deadline 时返回 `TimedOut`，已经找到的卡组仍是合法且精确评分的，但这时**不声称 Top-K 已证明完整**。同样，如果硬过滤后候选超过当前 512 张表示上限，或者紧凑元数据无法无损编码，会直接返回容量错误，不会为了继续运行而静默删卡。

形式化正确性分成两份文档：

- [**Exactness proof**](docs/exactness-proof.md)：定义可行集合、统一 Top-K 排序、各 solver 的完整性与 timeout / capacity 边界。
- [**Pruning proof**](docs/pruning-proof.md)：逐项证明每一种会真正删除搜索空间的规则，并给出对应代码位置；只负责排序或提供初始解的启发式会被明确排除在“剪枝”之外。

验证层用于找反例和防回归，而不是代替证明。目前固定门禁包括 256 × 15 = **3840** 条跨场景独立对照、针对历史反例和边界条件的专项测试，以及 native / server / Rust 1.89 / WASM / Node / Chrome 的跨运行时一致性检查。

## 许可证

[MIT](./LICENSE-MIT) OR [Apache-2.0](./LICENSE-APACHE)。
