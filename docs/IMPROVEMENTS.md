# allium-deck 改进空间与接手指南

> 面向**接手 allium-deck 开始修改**的工程师。读完能：理解架构、定位代码、明白当前三个关键问题的根因和改法。
>
> 配套：`docs/ARCHITECTURE.md`（逐文件地图）、`docker/`（standalone 快速迭代/验证环境）。
> 参考实现：`sekai-deck-recommend-cpp`（下称 moe），位于 `C:\Users\Administrator\Desktop\bot\参考仓库\sekai-deck-recommend-moe`。

---

## 0. 定位

allium-deck 是 moe（C++）的 Rust 重写，专攻 **DFS/B&B 精确搜索**，用 SoA + 位运算把搜索做到个位数 ms。

两个入口：
- **服务入口**：`allium-scapus/src/handlers/deck.rs::recommend` → `build_card_pool` → `search`。参数齐全（card_configs 等都构造）。
- **库/JSON 入口**：`engine.rs::recommend_json` → `parse_build_params_json`。**这是 standalone / 开源对外口，但参数解析不全**（见 §2）。

---

## 1. 三个关键问题（当前最高优先级）

| # | 问题 | 类别 | 根因位置 | 改动量 |
|---|------|------|---------|--------|
| **P1** | **WL 组卡完全没应用支援卡组加成** | 语义 bug | 数据未加载 / 无回归覆盖 | 中 |
| **P2** | **满级/满对话/满破/满技能/画布、固定卡等参数不生效** | 参数缺失 | `parse_build_params_json` 没解析 card_configs | 小 |
| **P3** | **建池 ~70ms（搜索只要个位数 ms）** | 性能 | 每张用户卡对 masterdata 做 O(N) 线性扫描 | 中 |

下面逐个给根因证据和改法。其余次要改进见 §5。

> **状态（2026-06，本轮 P1/P2/P3 均已实现并验证）**
> - **P2 已修复**：`engine.rs::parse_build_params_json` 现解析 `rarity_N_config` / `singleCardConfigs`（camelCase + snake_case），满级/满技能/满破/剧情/画布开关在 JSON/standalone 入口生效。新增 4 个解析单测。
> - **P3 已修复**：新增 `handler/index.rs`（`PoolIndexes`），建池开始对 masterdata 建一次 by-id 索引，`power.rs`/`skill.rs`/`mod.rs` 逐卡 O(N) 扫描改 O(1)。实测大账号（527 卡）建池 **91ms → 12ms（~7.5×）**，lib + e2e + CLI 大小账号对拍输出逐字节不变。
> - **P1 已修复**：moe 的 3 张 WL 支援静态表随 crate 携带（`crates/allium-deck/data/`），`engine.rs` 用 `include_str!` 内嵌兜底，masterdata 缺文件时回退。修了测试加载器类型漂移。实测 WL event 112 组卡 `support_deck` 非零 bonus 99 张（修复前恒 0）。新增内嵌表 + 回退单测。
> - 验证数据来源：CDN masterdata（`cdn.emptysekai.com/masterdata/cn/latest`）；standalone CLI 见 `docker/`。下文「根因/改法」保留作背景与后续维护参考。

---

## 2. P2：补齐参数（最快见效，先做）

### 2.1 根因（已核实）

`engine.rs::parse_build_params_json`（行 237-301）是 JSON / standalone 入口解析参数的唯一地方。它解析了：`region / eventId / eventType / liveType / target / musicId / musicDiff / fixedCards / fixedCharacters / excludedCards / worldBloomCharacterId / worldBloomEventTurn / challengeLiveCharacterId / eventUnit / eventAttr / filterOtherUnit / keepAfterTrainingState / bestSkillAsLeader / skillReferenceStrategy / liveSkillOrder / multiTeammate* / boost / otherScore / life / unitFilter / attrFilter`。

**它完全没有解析**：
- `card_configs`（即 `BuildParams.card_configs: CardConfigSet`）—— 所以 `rarity_N_config.{level_max, skill_max, episode_read, master_max, canvas, disable}` 全部停在默认 `false`。
- `single_card_configs`（单卡覆盖）。

后果：通过 JSON 入口传 "满级/满对话/满破/满技能/画布" 这些开关**被静默丢弃**，组卡按用户**实际**卡片等级算。`handler/card_config.rs::apply_card_config` 逻辑本身是好的，只是没人给它喂 config。

> 对照：服务入口 `deck.rs::build_params`（行 952-960）正确地从请求构造了 `CardConfigSet`。所以 bug 只在 JSON/standalone 入口。

固定卡/固定角色（`fixedCards`/`fixedCharacters`）**已解析**（行 256-257），若也报"不生效"，优先排查传入 JSON 的字段名（必须 camelCase `fixedCards`）和 `validate_fixed_constraints`（`handler/mod.rs:337`，超 5 个或角色重复会直接报错）。

### 2.2 改法

在 `parse_build_params_json` 里补一段把 JSON 的稀有度配置解析进 `params.card_configs` 和 `params.single_card_configs`。结构已存在（`handler/types.rs::CardConfigSet / CardRarityConfig / SingleCardConfig`），只需读 JSON：

```rust
// 伪代码，放进 parse_build_params_json
fn parse_card_config(v: &Value) -> CardRarityConfig {
    CardRarityConfig {
        disable: bool_field(v, "disable").unwrap_or(false),
        level_max: bool_field(v, "levelMax").or(bool_field(v,"level_max")).unwrap_or(false),
        skill_max: bool_field(v, "skillMax").or(bool_field(v,"skill_max")).unwrap_or(false),
        episode_read: bool_field(v, "episodeRead").or(bool_field(v,"episode_read")).unwrap_or(false),
        master_max: bool_field(v, "masterMax").or(bool_field(v,"master_max")).unwrap_or(false),
        canvas: bool_field(v, "canvas").unwrap_or(false),
    }
}
// params.card_configs.rarity_4_config = value.get("rarity4Config").map(parse_card_config).unwrap_or_default();
// ... rarity_1/2/3/birthday 同理
// params.single_card_configs = value.get("singleCardConfigs") 数组 → Vec<SingleCardConfig>
```

注意 `apply_card_config` 依赖 `master.max_level/max_skill_level/max_master_rank`，这些由 `enrich_master`（`handler/mod.rs:54`）从 `card_rarities` / `master_lessons` 回填，已就绪。

### 2.3 验证

加一个 mock case：同一张低级卡，分别带 `rarity4Config.levelMax=true` 和不带，断言 `pool.power_max` 不同。放 `tests/`（standalone Docker 跑），或直接单测 `recommend_json`。

### 2.4 顺带：moe 有而 allium 两个入口都缺的参数（按需补）

来自 moe `DeckRecommendOptions`（`sekai_deck_recommend.pyi`）的对照，下列 allium **完全没有**，属功能缺口（非本轮必须，列出供规划）：
- `member`（2-5 人队）—— 本轮**不做**（用户确认）。
- `target="bonus"` + `target_bonus_list` —— 目标加成组卡（moe `find-target-bonus-cards-dfs.cpp`）。
- `support_master_max` / `support_skill_max` —— 算支援卡组 bonus 时强制满破/满技能。
- `custom_bonus_character_ids` / `custom_bonus_attr` / `custom_bonus_character_support_units` —— 自定义混活加成（allium 有 `event_unit/event_attr` 的简版，但缺 character_ids 列表和 VS 应援约束）。
- `skill_order_choose_strategy` + `specific_skill_order` —— allium 内部 `LiveSkillOrder` 支持，但服务/JSON 入口写死 `Best`，没暴露。
- `forcedLeaderCharacterId` —— 终章强制队长。

---

## 3. P1：WL 组卡完全没应用支援卡组加成（头号 bug）

### 3.1 根因（已核实，证据链完整）

世界开花（WL）每个 deck 除了 5 张主队卡，还有一组「支援卡组」（WL1=12 张 / WL2=20 / WL3=25），它们贡献一笔可观的 `support_deck_bonus_rate`。allium 这笔加成**恒为 0**，根因是支援加成查找表**从未被加载**：

1. **moe 把 3 张 WL 支援加成表当作仓库静态数据**，打包在 `data/worldBloomSupportDeckBonusesWL{1,2,3}.json`（moe `master-data.cpp:263-268` 从 `./data/` 单独加载，注释明说「不随 masterdata 更新」）。
2. **allium-deck 没有携带这 3 张表**。`engine.rs::OwnedGameData::load`（行 603-609）用 `load_optional_json` / `load_wl3_support_bonuses` 从 **masterdata 目录**读它们 —— 但生产 masterdata 同步（`allium-scapus/src/state.rs::load_deck_game_data`，行 286-327）只下载 S3 masterdata 前缀下的 `.json`，**不包含这 3 张静态表**。
3. `load_optional_json`（`engine.rs:980`）**文件不存在就静默返回空 `Vec`**，不报错。
4. 下游 `handler/support_bonus.rs::support_bonus_table`（行 71-78）按 turn 取到**空切片**，`calc_wb_support_bonus`（行 32-37）`find` 失败 → **静默 `return 0.0`**。
5. 于是 `handler/mod.rs::build_support_deck`（行 449-477）给每张卡算出的 bonus 都是 0，`SupportDeck.cards` 全 0；`search/evaluate.rs::calc_support_bonus`（行 1044，**这才是真正生效的搜索期计算**）加出来也是 0。
6. **回归测试掩盖了它**：`tests/testdata_adapter/masterdata_loader.rs:222-224` 把 `wb_support_deck_bonuses_wl1/2/3` 直接**硬编码成空 `Vec`**，所以 e2e 从不覆盖 WL 支援，bug 长期无人发现。

> 注意：`src/eval/event_bonus.rs` 里也有一份 WL 支援逻辑，但它在 `#[cfg(any())]` 下**不参与编译**（`lib.rs:6-7`，TASK-011 前的旧 eval 层）。真正跑的是 `search/evaluate.rs::calc_support_bonus` + `handler/mod.rs::build_support_deck`。改的时候别改错文件。

### 3.2 改法（分两步）

**第一步：让这 3 张表进得来（必做）。**
- 选项 a（推荐，跟 moe 一致）：把 `data/worldBloomSupportDeckBonusesWL{1,2,3}.json` 作为**静态资源随 crate 携带**。放到 `crates/allium-deck/data/`，用 `include_str!` 在 `OwnedGameData::load` 里兜底——masterdata 目录没有就用内嵌副本。这样开源/standalone 也能用。
- 选项 b：把 3 张表纳入 S3 masterdata 同步前缀，运维侧补齐。但 standalone 跑不到，且依赖外部数据管线，脆弱。
- 必须同时修 `tests/testdata_adapter/masterdata_loader.rs:222-224`：从静态副本真正加载，否则回归依旧测不到。

**第二步：加回归 case（必做，否则无法验收）。**
- moe 提供 `get_world_bloom_support_cards(options)`（`sekai_deck_recommend.pyi:438`）可单独导出支援卡组及其 bonus。用它对一个 WL 活动 + 角色生成金标准，断言 allium `build_support_deck` 产出的 `(card_id, bonus)` 列表与 moe 一致。
- 端到端：跑一个 WL event 的 score 组卡，断言结果 deck 的 `support_deck_bonus_rate > 0` 且与 moe 输出一致。`testdata/real/` 已有 `_test_world_bloom_*` 和 `scenario_final_chapter_cpp_reference.json`，确认它们的 cpp_output 是否真带支援 bonus；若是，把它们接进 e2e combo。

### 3.3 数据流核对清单（改完逐项验证）

| 环节 | 文件:行 | 期望 |
|------|---------|------|
| 加载 WL 表 | `engine.rs:603-609` | 3 张表非空 |
| 测试加载器 | `masterdata_loader.rs:222-224` | 不再硬编码空 Vec |
| turn 解析 | `handler/event_bonus.rs:81-112` | WL1/2/3 → count 12/20/25 |
| 单卡 bonus | `handler/support_bonus.rs:4-69` | specific/others + masterRank + skillLevel + unitEventLimited 累加 |
| 建支援组 | `handler/mod.rs:435-497` | `SupportDeck.cards` 含非零 bonus，按 bonus 降序 |
| 搜索期加成 | `search/evaluate.rs:1044 calc_support_bonus` | 选 top-`count` 张不与主队重复的卡，加和 |
| 终章 by-character | `handler/mod.rs:480-497` | 每个 leader 角色一套支援组 |
| WL3 power cap | `handler/mod.rs:585-593` | turn==3 → 336000 |

### 3.4 `support_master_max` / `support_skill_max`（顺带补）

moe 算支援卡组 bonus 时可强制把支援卡视作满破/满技能（`DeckRecommendOptions.support_master_max/skill_max`）。allium 的 `build_support_deck` 直接用卡的真实 `master_rank/skill_level`（`handler/mod.rs:459-461`）。补法：在 `BuildParams` 加两个 bool，传到 `calc_wb_support_bonus` 时覆盖 `master_rank/skill_level`。小改动，但影响支援 bonus 数值，需与 moe 对齐后再上。

---

## 4. P3：建池 ~70ms（搜索只要个位数 ms）

### 4.1 根因（已核实）

`build_card_pool`（`handler/mod.rs:631`）对**每张用户卡**、**每个技能状态**（花前/花后两份）都重新对 masterdata 做**线性 `find` / `filter` 扫描**。masterdata 的 `cards` 有上千行、`card_parameters` 有数万行（每卡 ~60 级一行），用户卡也有几百张，于是建池是 `O(用户卡数 × masterdata 行数)` 的笛卡尔扫描。

逐个热点（按代价排序）：

| 热点 | 文件:行 | 每次扫描 | 调用频次 |
|------|---------|---------|---------|
| 按 cardId 找 master | `mod.rs:652-655` `game.cards.iter().find` | 整个 cards 表 | 每张用户卡 |
| 等级参数 | `power.rs:50-55` `card_parameters.iter().filter().max_by_key` | **整个 card_parameters 表（最大，数万行）** | 每张卡 |
| 剧情加成 | `power.rs:64-67` `card_episodes.iter().filter` | 整个 episodes 表 | 每张卡 |
| 突破加成 | `power.rs:75-78` `master_lessons.iter().filter` | 整个 lessons 表 | 每张卡 |
| 画布加成 | `power.rs:92-95` `canvas.iter().find` | canvas 表 | 每张卡（有画布时） |
| area item | `power.rs:146` `area_item_levels.iter().filter` | 整个 area 表 | **每张卡 × 6 unit × 4 member_key**（power.rs 双重循环） |
| 角色 rank | `power.rs:118-122` `character_ranks.iter().filter().max_by_key` | rank 表 | 每张卡 |
| 技能 | `skill.rs:56-59` `skills.iter().find` | 整个 skills 表 | 每张卡 × 技能状态 |
| 技能效果 | `skill.rs:64-67` `skill_effects.iter().filter` | 整个 effects 表 | 每张卡 × 技能状态 |
| 活动卡 bonus | `event_bonus.rs` 多处 `event_cards/deck_bonuses.iter()` | 各活动子表 | 每张卡 |
| honor | `mod.rs:610-613` 双重 `find` | honors 表 | 每个用户称号 |

其中 **`power.rs:50` 扫 `card_parameters`** 几乎肯定是头号开销：该表是「卡 × 等级」展开，行数最大，且每张卡都全表 filter 一遍。

`build_support_deck`（`mod.rs:435`）对 `support_cards`（= 全部 intermediate，含花前花后两份）再扫一遍算 WL 支援，终章还按 26 个角色各跑一次（`mod.rs:492`）——WL/终章建池更慢。

### 4.2 改法（按性价比排序）

**改法 1（最高收益，推荐先做）：预建索引，把 O(N) 查表降到 O(1)。**
在 `OwnedGameData` 里（或 `build_card_pool` 开头一次性）构建 `HashMap` / 排序数组索引：
- `card_by_id: HashMap<i32, &MasterCard>`
- `params_by_card: HashMap<i32, Vec<&CardParameter>>`（或按 card_id 分组后内部按 level 排序，查询用二分）
- `episodes_by_card`, `lessons_by_rarity`, `skill_by_id_level: HashMap<(i32,i32), &Skill>`, `effects_by_skill_level`, `canvas_by_rarity`, `rank_by_char`, `event_cards_by_card`...

这些索引**只依赖 masterdata**，masterdata 在服务里是 `Arc<OwnedGameData>` 长期不变（`state.rs`），所以**索引应建一次并随 `OwnedGameData` 缓存**，而不是每次 `build_card_pool` 重建。最干净的做法：给 `OwnedGameData` 增加一个 `Indexes` 字段（`OnceCell` 懒构建），`GameData<'a>` 借用它。

预期：把建池从 70ms 降到与 moe 同量级（moe 用 `CardDetailMap` + 预聚合，单账号建池在毫秒级）。

**改法 2：花前花后两份只在必要时算。**
`skill_states_for_card`（`mod.rs:413`）在非 `keep_after_training_state` 且卡有特训技能时返回**两份**（After+Before），power 部分却完全相同——但当前每份都走完整 `CardIntermediate` 构造并 push 进 `support_cards`（`mod.rs:723`），WL 支援计算量翻倍。可只对 skill 维度分裂，power/event_bonus 复用。

**改法 3：per-character trim 提前。**
现在是先对所有卡算完整 power/skill（最贵的部分），**之后**才 `per_character_trim`（`mod.rs:144`）裁剪到每角色 N 张。可以先用便宜的 key（稀有度 + 等级上限估计）粗筛，再只对存活卡算精确 power。但要小心别破坏 trim 依赖的 `power_max`——需要一个便宜的 power 上界估计。

**改法 4：并行建池（rayon）。**
每张卡的 `CardIntermediate` 构造彼此独立，可 `par_iter`。allium-scapus 已依赖 rayon（根 Cargo.toml:48）。但注意 `crates/allium-deck/Cargo.toml` 目前**只依赖 serde/serde_json/thiserror**，加 rayon 会让 standalone 构建多一个依赖——可放 feature gate。**先做改法 1**，索引化后建池可能已够快，未必需要并行。

### 4.3 验证

- `tests/e2e_regression.rs` 已有计时（`measure_*_pruning_stats` 打印 `total_elapsed_ms`，但那是 search 计时）。给 `build_card_pool` 单独加一个 bench：固定一个 ~400 张卡的 user fixture，循环 100 次取均值。standalone Docker 跑。
- 改完务必重跑全部 e2e combo，确认数值**不变**（索引化是纯重构，输出必须逐字节一致）。

---

## 5. 次要改进（中期）

- **C1 搜索并行**：`final_chapter.rs::search_auto_leaders_two_phase`（`mod.rs` 路径）按 leader 角色分 job，天然可并行；普通 DFS 可按首卡分片。当前搜索已是个位数 ms，优先级低于建池。
- **C2 GA/SA 兜底**：moe 默认 GA，allium 只有 DFS。大池或超时场景下 DFS 可能跑不完（虽有 30s timeout + warm_start 兜底）。moe 的 `find-best-cards-ga.cpp` / `find-best-cards-sa.cpp` 可作蓝本。工作量大，仅当线上出现 DFS 超时再考虑。
- **C3 组分技能精度**：`types.rs::SkillLookup` 的 skill_key 只 2 档（全 5 人 / 非 5 人），2-4 人同组的上界/支配剪枝用保守低估值（叶子精确评估是准的）。仅在「最优解恰为 2-4 张同组」时可能漏解，罕见。详见 `types.rs:185-215` 注释。
- **C4 `event_rarity_bonus_rates` 膨胀**：`engine.rs:660-675` 把稀有度 bonus 表对所有 event_id 做笛卡尔积展开，event 多时这张表很大，拖慢加载和 `event_bonus.rs` 的扫描。可改成按 (event_id, rarity, master_rank) 索引、按需查。
- **C5 `skill_order_choose_strategy` / `forcedLeaderCharacterId` 暴露**：内部已支持 `LiveSkillOrder::{Best,Worst,Average,Specific}` 和 `specific_skill_order`，但 `deck.rs::build_params` 写死 `Best`、JSON 入口也不解析。补透传即可。

---

## 6. 动手顺序建议

1. **P2 参数**（半天）：`parse_build_params_json` 补 card_configs 解析 + 单测。立即让"满级/满对话"在 standalone 生效。
2. **P3 建池索引化**（1-2 天）：给 `OwnedGameData` 加懒索引，改 `power.rs`/`skill.rs`/`event_bonus.rs`/`mod.rs` 的查表。纯重构，e2e 输出必须不变。
3. **P1 WL 支援**（1-2 天）：内嵌 3 张静态表 + 修测试加载器 + 加 WL 回归 case。需要 moe 生成金标准。
4. 之后按需做 §5。

每步改完都用 `docker/` 跑 `cargo test`（含 e2e 回归）验收。
