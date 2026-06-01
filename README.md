# allium-deck

Project Sekai 组卡推荐引擎的 Rust 实现，专攻 **DFS / 分支限界（B&B）精确搜索**。

给定玩家卡组、活动加成与目标（综合力 / 技能 / 活动点数 / MySekai 等），在巨大的组合空间里搜出最优的 5 张卡编成。核心数据结构用 SoA（结构体数组）+ 位运算组织，配合角色感知的后缀上界与支配剪枝，把单次搜索压到亚毫秒级。

## 关于实现来源

游戏内的各项数值公式（综合力、技能加成、活动点数、支援卡组加成等）与参数设计，参考了社区既有的两个 C++ 参考实现（代号 cpp / moe）。这些游戏机制本身是确定的，源码注释中保留了对照出处，便于核对与后续维护。

在此基础上，本实现并非逐行翻译，而是对**底层热路径与搜索剪枝做了彻底的 Rust 重构**，核心数据结构全部按 cache line 对齐：

- **建池（pool building）**：对 masterdata 一次性建立 by-id 索引，把逐卡 O(N) 的线性扫描降为 O(1) 查表。
- **搜索（search）**：SoA 卡池 + 512-bit 候选位图、综合力压成 u18×8 槽位 + 查找表、逐角色支配裁剪、角色感知后缀上界、贪心 + 1-swap warm start 下界，使分支限界尽早剪枝。

`CardPool` 采用列式 SoA 布局，每列 64 字节对齐。典型候选池（130–260 张卡）整体 **~7–12 KB**，加上 `SearchContext`、`SuffixBound` 等搜索期结构，**全部热路径数据落在 L1 data cache 内**（EPYC 9K85 单核 L1d = 32 KB）。叶子评估遍历卡组时，访问模式是逐列顺序扫描，每次只碰当前列的一个 cache line——无 TLB miss，无跨行颠簸。

> 性能（生产环境实测，AMD EPYC 9K85 单核，release profile：`opt-level=3` / `lto="fat"` / `codegen-units=1` / `target-cpu=znver5`）：
> 典型账号单次**建池约 6 ms**（结果可缓存命中），**搜索亚毫秒级**。
> 数值随账号规模、目标（综合力/技能/分数）与活动类型波动。

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

性能数字应在 release profile 下测量。`docker/` 下提供 standalone CLI（`recommend_cli`），打印分阶段耗时（建池 vs 搜索），方便快速迭代验证。

## 静态数据

`data/` 内嵌 3 张世界开花（World Bloom）支援卡组加成表。这些表在参考实现中作为仓库静态资源随包携带、不随 masterdata 更新，因此这里用 `include_str!` 内嵌，masterdata 缺失对应文件时回退使用。

## 测试

- 单元测试：散落各模块 `#[cfg(test)]`（pool / search / handler）。
- 端到端回归：`tests/e2e_regression.rs` 读 manifest 驱动，将搜索结果与参考输出逐项比对。数据路径可由环境变量覆盖（`ALLIUM_TESTDATA` / `ALLIUM_MASTERDATA_CN` / `ALLIUM_MASTERDATA_JP` / `ALLIUM_MUSIC_METAS`）。
- 搜索结果正确性由以下测试用例用**暴力枚举**验证：
  - `search_dfs_matches_bruteforce_for_best_deck`
  - `search_dfs_bonus_noevent_matches_bruteforce_with_suffix_max_break`
  - `search_dfs_mysekai_matches_bruteforce_with_suffix_max_break`
  - `search_suffix_bound_is_sound_and_zero_pool_is_zero`
  - `search_dominance_preserves_best_score`

## Soundness

每个剪枝机制经代码审计和暴力枚举测试验证。

**支配剪枝（`dominance.rs`）——Sound ✅**

只比较**同一角色**内的两张卡（`pool.char_id(a) == pool.char_id(b)`）。淘汰 B 的前提是 A 在以下所有维度上 ≥ B：

- 8 种编队组合的综合力（逐槽 u18 解码后比较）
- 技能（同类型才可比；Score Up 比数值，Unit Count 比同 unit 的各人数加成，Diff 比 base 和 increment，Ref 比 rate 和 max）
- 活动加成（base 和 limited 分别比较）
- 属性相同（否则对 diff-attr 奖励的贡献不同，不能断言 B 无害）
- Unit mask 是超集（rhs_mask ⊆ lhs_mask，避免丢失候选编队）

替换安全：把 B 换成 A，在任何目标下分数不降。World Bloom 活动下支配剪枝**完全关闭**，因为 diff-attr 和支援卡组的交叉约束无法在单卡层面保证单调性。

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

[AGPL-3.0-only](./LICENSE)。Copyright (C) allium / emptysekai。
