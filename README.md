# allium-deck

Project Sekai 组卡推荐引擎的 Rust 实现，专攻 **DFS / 分支限界（B&B）精确搜索**。

给定玩家卡组、活动加成与目标（综合力 / 技能 / 活动点数 / MySekai 等），在巨大的组合空间里搜出最优的 5 张卡编成。核心数据结构用 SoA（结构体数组）+ 位运算组织，配合角色感知的后缀上界与支配剪枝，把单次搜索压到亚毫秒级。

## 关于实现来源

游戏内的各项数值公式（综合力、技能加成、活动点数、支援卡组加成等）与参数设计，参考了社区既有的两个 C++ 参考实现（代号 cpp / moe）。这些游戏机制本身是确定的，源码注释中保留了对照出处，便于核对与后续维护。

在此基础上，本实现并非逐行翻译，而是对**底层热路径与搜索剪枝做了彻底的 Rust 重构**，核心数据结构全部按 cache line 对齐：

- **建池（pool building）**：对 masterdata 一次性建立 by-id 索引，把逐卡 O(N) 的线性扫描降为 O(1) 查表。
- **搜索（search）**：SoA 卡池 + 512-bit 候选位图、综合力压成 u18×8 槽位 + 查找表、逐角色支配裁剪、角色感知后缀上界、贪心 + 1-swap warm start 下界，使分支限界尽早剪枝。

`CardPool` 采用列式 SoA 布局，每列 64 字节对齐。典型候选池（130–260 张卡）整体 **~7–12 KB**，加上 `SearchContext`、`SuffixBound` 等搜索期结构，热路径数据适合驻留在现代服务器 CPU 的 L1 data cache 内（EPYC 9K85 每核 L1d = 48 KiB）。叶子评估遍历卡组时按列顺序访问，尽量减少无关 cache line 和 TLB 压力。

> 性能（AMD EPYC 9K85，固定单核，release profile：`opt-level=3` / `lto="fat"` / `codegen-units=1` / `target-cpu=znver5`）：masterdata 常驻内存时，完整建池 cache miss（包含当前用户和参数的准备）典型约为 **0.6 ms**。10 个不同账号的多人活动 Top-8 搜索中，账号均值的纯算术平均为 **0.3270 ms**，范围为 **0.1017–0.8369 ms**。在这一典型场景中，`v0.0.6` 相比 `v0.0.5` 的建池约快 **20×**，搜索约快 **5–10×**。
>
> 建池数字不包含 masterdata 文件读取或 JSON 解析。x86-64 在运行时检测 AVX-512F/BW；不支持的 CPU 和其他架构自动使用 scalar fallback。实际耗时会随账号规模、活动规则、目标和候选池变化；20× 建池和 5–10× 搜索提升描述仅针对典型多人活动组卡，不是所有模式的性能保证。

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

## 模块地图

| 模块 | 职责 |
| --- | --- |
| `engine` | 对外入口（`recommend_json` / `recommend`）、masterdata 加载（`OwnedGameData`）、JSON 参数解析 |
| `types` | 公共类型与枚举（`Unit` / `Attr` / `LiveType` / `ScoreTarget`）、综合力/技能查找表 |
| `handler` | 建池层：候选裁剪、综合力/技能/活动加成预计算、WL 支援卡组、构建搜索上下文 |
| `pool` | SoA 卡池：列式存储、位图、对齐布局、冻结后只读 |
| `search` | 搜索层：支配剪枝、后缀上界、warm start、按目标/场景分派的 B&B、叶子精确评估 |

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
          ├─ 候选裁剪（按活动点数 / 逐角色）
          ├─ 排序灌入 SoA CardPool
          └─ 构建 SearchContext（含 WL 支援卡组）
                          │
                          ▼  (CardPool, SearchContext)
        search (search)                        // 搜索
          ├─ 逐角色支配裁剪
          ├─ 角色感知后缀上界（B&B 剪枝核心）
          ├─ warm start 下界（贪心 + 1-swap）
          └─ 按 target / 场景分派 DFS：
               Power / Skill / Score / MySekai / challenge / 终章
                          │
                          ▼
                  Vec<DeckResult> → JSON
```

## 依赖与构建

只依赖 `serde` / `serde_json` / `thiserror`，无图形/异步/系统库依赖，可独立秒级编译：

```bash
cargo build --release
```

性能数字应在 release profile 下测量。`src/bin/recommend_cli.rs` 提供 standalone CLI（`cargo install allium-deck` 出来后命令名 `recommend_cli`），打印分阶段耗时（建池 vs 搜索），方便快速迭代验证。

## CLI

`recommend_cli` 是可独立运行的组卡推荐命令行工具，可从 [GitHub Releases](https://github.com/empty-sekai/allium-deck/releases) 下载预编译二进制，或从源码安装。

从命令行跑一次完整推荐，打印建池/搜索耗时和 Top-K 卡组：

```bash
# 方式1: 下载预编译二进制 (以 linux-x86_64 为例)
curl -L -o recommend_cli \
  https://github.com/empty-sekai/allium-deck/releases/download/v0.0.6/recommend_cli-v0.0.6-linux-x86_64
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

## 静态数据

`data/` 内嵌 3 张世界开花（World Bloom）支援卡组加成表。这些表在参考实现中作为仓库静态资源随包携带、不随 masterdata 更新，因此这里用 `include_str!` 内嵌，masterdata 缺失对应文件时回退使用。

## 测试

- 单元测试：散落各模块 `#[cfg(test)]`（pool / search / handler）。
- Eval fixtures：`tests/fixtures/eval` 使用小型可审计数据固定火数、协力/Cheerful、技能顺序、WL、MySekai 等评分规则。
- 搜索结果正确性由以下单元测试用**暴力枚举**验证：
  - `search_dfs_matches_bruteforce_for_best_deck`
  - `search_dfs_bonus_noevent_matches_bruteforce_with_suffix_max_break`
  - `search_dfs_mysekai_matches_bruteforce_with_suffix_max_break`
  - `search_suffix_bound_is_sound_and_zero_pool_is_zero`
  - `search_dominance_preserves_best_score`
- `tests/benchmark_proof.rs` 用无剪枝暴力枚举对照正式搜索，含三个暴力对照测试和一个数据集校验测试：
  - `rust_bruteforce_matches_exact_on_full_testdata_pools`（暴力对照）：从本仓库小型 fixtures 抽样，按原输入构建完整卡池，对正式搜索与暴力枚举比较结果。只选择完整卡池组合数不超过 `ALLIUM_BF_CANDIDATE_LIMIT` 的 fixture。
  - `rust_bruteforce_matches_exact_top_k_on_issue2_fixture`（Top-K 回归）：锁住 [issue #2](https://github.com/empty-sekai/allium-deck/issues/2) 的 fixture `real/mass_392500_score_multi_ev`，验证普通主搜索路径的 Top-K 支配替代展开。该池组合数约 7000 万，需以 `ALLIUM_BF_TOP_K=3 ALLIUM_BF_CANDIDATE_LIMIT=100000000` 运行。
  - `rust_bruteforce_matches_exact_on_large_filtered_pools`（暴力对照）：针对高练度大卡池。先丢弃 1/2 星卡（`ALLIUM_BF_MIN_RARITY`），再按角色对 power / skill / event-bonus 各维度保留前 N 张（`ALLIUM_BF_PER_CHAR_KEEP`），把卡池压到可暴力枚举的规模，再做暴力对照。这是覆盖高练度高价值候选区的 stress 子集，不声称是完整大卡池的证明：被裁掉的低价值卡仍可能进入某些 Top-K 次优解。
  - `testdata_corpus_layers_are_classified`（数据集校验）：核对当前 fixture 清单分层与目标分布，输出 `target/benchmark-proof/report.md` 与 JSON 明细。
- 相关环境变量：`ALLIUM_BF_TOP_K`、`ALLIUM_BF_CASE_LIMIT` / `ALLIUM_BF_LARGE_CASE_LIMIT`、`ALLIUM_BF_CANDIDATE_LIMIT` / `ALLIUM_BF_LARGE_CANDIDATE_LIMIT`、`ALLIUM_BF_MIN_RARITY`、`ALLIUM_BF_PER_CHAR_KEEP`。缺少 masterdata 时这些对照测试会跳过。

## Soundness

每个剪枝机制经代码审计和暴力枚举测试验证。

**支配剪枝（`dominance.rs`）——Sound ✅**

只比较**同一角色**内的两张卡（`pool.char_id(a) == pool.char_id(b)`）。淘汰 B 的前提是 A 在以下所有维度上 ≥ B：

- 8 种编队组合的综合力（逐槽 u18 解码后比较）
- 技能（同类型才可比；Score Up 比数值，Unit Count 比同 unit 的各人数加成，Diff 比 base 和 increment，Ref 比 rate 和 max）
- 活动加成（base 和 limited 分别比较）
- 属性相同（否则对 diff-attr 奖励的贡献不同，不能断言 B 无害）
- Unit mask 是超集（rhs_mask ⊆ lhs_mask，避免丢失候选编队）

替换安全：把 B 换成 A，在任何目标下分数不降。World Bloom 活动同样走支配剪枝；支援卡组独立保存在 `SearchContext`，不会因为主搜索池压缩而丢支援候选，且支配关系要求属性相同，因此 diff-attr 奖励不会被异色替换破坏。

Top-K（`top_k > 1`）下被支配卡参与的组合本身可能是合法的次优解，仅靠裁剪会丢名次（曾为 [issue #2](https://github.com/empty-sekai/allium-deck/issues/2)）。普通主搜索路径现在会在裁剪时记录支配映射（链压缩到存活根），搜索后对每个结果做**替代回换展开**：设真实 Top-K 中有含被裁卡的卡组 D，把其中每张被裁卡换成支配根得到 D'，由支配性 score(D') ≥ score(D) ≥ 第 K 名阈值，D' 必在裁剪池的精确 Top-K 里；从 D' 逐槽（含多槽组合）把支配根换回被裁卡并重新评估、合并，即可还原该路径下丢失的次优解。回换分数单调不升，按当前第 K 名阈值剪枝；`top_k = 1` 跳过展开，主搜索路径零开销。终章 member 侧还有额外裁剪，Top-K 替代展开另见 [issue #7](https://github.com/empty-sekai/allium-deck/issues/7)。

**后缀上界（`suffix.rs`）——Sound ✅**

上界计算的核心是**角色感知聚合**：按 power / skill / bonus 三个维度分别对 27 个角色取单卡最大值，然后取未使用角色中 top-N 求和。因为每角色至多选一张卡，任何实际编队的各维度总和都不可能超过所在维度的 top-N 角色最大值的和。三个维度的 top-N 可能取到不同角色，这是保守高估，不会漏解。

在此基础上做了多层收紧：

- **Exclusion delta**：当选了一张卡后，将该角色从 suffix 中排除，重新降级到下一个可用角色。获取 ex-lusion delta 时完全绕过分支，用 compact bit index + popcount 直接定位。
- **Dense suffix tail**：从 SoA 右端向左扫描，根据实际出现的角色单调收窄。随 DFS 位置推进，`ceiling(i+1) ≤ ceiling(i)`，一旦跌到阈值以下可以安全 break 整层。
- **World Bloom extra bound**：在 support deck 和 diff-attr 上限上加额外一层 ceiling，取各维度最紧值。
- **叶子评估**（`evaluate.rs`）使用实际的同组人数 `unit_counts[unit].clamp(1, 5)` 查表，1-5 人效果为精确值，非近似。

**Power / Skill 路径——不保证最优 ❌**

`search_instrumented`（`mod.rs:37-38`）对 Power 和 Skill 目标不走完整 B&B，而是取排序后前缀（Power 28 张每角色 ≤6，Skill 20 张每角色 ≤3）内枚举。前缀外的卡被直接丢弃，没有上界证明能安全裁剪——纯粹的性能取舍。

实际风险很低：Power / Skill 是纯加性目标，没有跨卡技能协同，每角色的最优卡就是 power_max / skill_max 最高的那张。一个角色有 4 张以上技能卡、且最优解必须用第 4 张的情况极端罕见。但这不是形式化保证。

**小结**

| 目标 | 算法 | Sound | 说明 |
| --- | --- | --- | --- |
| Score / Mysekai | 完整 B&B | ✅ | 支配剪枝 + 角色感知后缀上界 + 多层收紧 |
| 终章 | 角色分组 + B&B | ✅ | leader × member 两段 DFS |
| Challenge | 暴力枚举 | ✅ | 无剪枝，仅 game_id 去重 |
| Power / Skill | 前缀 DFS | ❌ | 28/20 张限制，每角色 6/3 上限 |

## 许可证

[MIT](./LICENSE-MIT) OR [Apache-2.0](./LICENSE-APACHE)。
