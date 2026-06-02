# allium-deck 架构地图

> 配合 `IMPROVEMENTS.md` 阅读。本文给出**逐文件职责**和**数据流**，让你快速定位"要改的东西在哪"。

## 总体数据流

```
masterdata JSON ─┐
                 ├─→ OwnedGameData::load (engine.rs)  // 一次性，服务里 Arc 缓存
music_metas ─────┘        │
                          ▼  as_ref()
                    GameData<'a>  (handler/types.rs，只读借用视图)
user JSON ──→ parse_user_profile_json (engine.rs) ──→ UserProfile
params JSON ─→ parse_build_params_json (engine.rs) ──→ BuildParams
                          │
                          ▼
        build_card_pool (handler/mod.rs)         ←── 「建池」，~70ms，见 IMPROVEMENTS §4
          ├─ build_event_context (event_bonus.rs)
          ├─ 每张用户卡:
          │    ├─ apply_card_config (card_config.rs)   // 满级/满技能等开关
          │    ├─ build_power (power.rs)               // 综合力 4×6 预计算表
          │    ├─ build_card_event_bonus (event_bonus.rs)
          │    └─ build_skill (skill.rs)               // 花前/花后两份
          ├─ ep_prefilter / per_character_trim (mod.rs)  // 候选裁剪
          ├─ sort_and_gather (gather.rs)               // 排序 + 灌进 SoA CardPool
          └─ build_search_context (mod.rs)             // SearchContext + WL 支援组
                          │
                          ▼  (CardPool, SearchContext)
        search (search/mod.rs)                    ←── 搜索，个位数 ms
          ├─ eliminate_dominated (dominance.rs)        // 逐角色支配裁剪
          ├─ SuffixBound::build (suffix.rs)            // 角色感知后缀上界
          ├─ warm_start_best (warm_start.rs)           // 贪心+1-swap 下界
          └─ 按 target/场景分派:
               ├─ search_simple_target (mod.rs)        // Power/Skill
               ├─ challenge_search.rs                  // challenge（不强制角色唯一）
               ├─ final_chapter.rs                     // 终章（leader×member 两段式）
               └─ dfs.rs                               // 通用 Score/Mysekai B&B
                          │
                          ▼
                  Vec<DeckResult>  (search/types.rs) → JSON / 渲染
```

## 模块逐文件

### 顶层
- `lib.rs` —— 模块声明。注意 `eval` 在 `#[cfg(any())]` 下**不编译**（旧层，别改）。
- `types.rs` —— 公共类型：`Unit/Attr/LiveType/ScoreTarget/...` 枚举，`PowerDetail/SkillInfo/PowerLookup/SkillLookup`（综合力/技能查找表，含精度与槽位编码注释），`EventContext/DeckContext`。**改组分技能精度看这里**（`SkillLookup`，IMPROVEMENTS §5 C3）。
- `engine.rs` —— 对外入口 + masterdata 加载。
  - `recommend_json` / `recommend`：库入口。
  - `parse_user_profile_json` / `parse_build_params_json`：JSON→结构体。**P2 缺参数在这里**（§2）。
  - `OwnedGameData::load` + `Raw*` 结构体：从磁盘读 masterdata 并转内部表。**P1 的 WL 表加载在这里**（行 603-609）。

### handler/（建池层，最值钱的优化目标）
- `mod.rs` —— `build_card_pool` 总编排；候选裁剪（`ep_prefilter_keep` / `per_character_trim` / `target_per_character_trim`）；`build_search_context`；`build_support_deck`（WL 支援，P1）；`compute_honor_bonus`。
- `types.rs` —— `GameData<'a>`（只读视图）、`UserProfile`、`BuildParams`、`CardConfigSet`、所有 masterdata/userdata 结构体、`parse_unit_code/parse_attr_code` 等映射。**加新参数先动 `BuildParams`**。
- `card_config.rs` —— `apply_card_config`：把满级/满技能/满破/剧情/画布开关施加到 `UserCard`。逻辑正确，P2 只是没人喂它 config。
- `power.rs` —— `build_power`：综合力。base/character(f32)/areaItem(f64)/fixture/gate。**P3 热点密集区**。
- `skill.rs` —— `build_skill`：解析 score_up / unit_count(组分) / diff(异团) / reference(吸分) / character_rank / life，产出 `SkillResult` + 侧表项。
- `event_bonus.rs` —— `build_event_context`（活动类型/turn/WL角色/fake event 等）、`build_card_event_bonus`（单卡活动加成 + 角色/属性轴命中标记）、`build_leader_honor_bonus`/`build_leader_limit_bonus`（终章）。
- `support_bonus.rs` —— `calc_wb_support_bonus`：WL 支援单卡 bonus。**P1：表为空就静默返回 0**。
- `music.rs` —— `build_music_params`：歌曲倍率/系数。
- `gather.rs` —— `sort_and_gather`：按 target 排序候选卡，`encode_power`（综合力压成 u18×8 槽 + LUT），灌进 `PoolBuilder`，输出 `CardPool` + `FullPrecisionCard`。

### pool/（SoA 卡池，搜索热数据结构）
- `types.rs` —— `CardIdx`、`SkillSlot`（skill_type 0/1/2/3 = 普通/组分/异团/吸分）、`EventBonusHot`（bonus 以 0.5% 为单位的 u8）、`UnitCountSkill/DiffSkill/RefSkill` 侧表、`Mask`（512-bit 候选位图）、`SpecialTables`。
- `layout.rs` —— `PoolLayout`：SoA 各列的 64 字节对齐偏移。
- `builder.rs` —— `PoolBuilder`：可写构建期，`set_*` 填列 + `mark_char/unit/attr` 置位图，`freeze()` 冻结。
- `card_pool.rs` —— `CardPool`：只读 SoA。`power_values/power_lut/skill/event_bonus/char_id/attr/unit_mask_raw/game_id/power_max/skill_min/skill_max` 列访问器；`compact(keep)` 按位图重打包（dominance 后用）。

### search/（搜索层）
- `mod.rs` —— `search` 总入口；按 target/场景分派；`search_simple_target`（Power/Skill 的前缀+per-char-cap DFS）；`remap_results`（dominance 压缩后索引还原）。
- `context.rs` —— `SearchContext`（搜索期不变量）、`SupportDeck`、`remap`（按 keep 位图重映射 per-card 向量）。
- `evaluate.rs` —— **叶子精确评估**：`leaf_evaluate_checked`（按 target 编码排序值）、`calc_live_score` / `calc_event_point` / `calc_mysekai_internal`、`resolve_card_power`（按真实同组/同色人数查 u18 LUT）、技能排列枚举（吸分 ref 的 mask 枚举）、`calc_support_bonus`（**WL 支援搜索期加成，P1 真正生效处**）。
- `suffix.rs` —— `SuffixBound`：角色感知后缀上界 + dense 后缀 tail，B&B 剪枝的数学核心。`ceiling()` 是各 target 的统一上界函数。
- `dominance.rs` —— `eliminate_dominated`：逐角色支配裁剪（power 8 槽 + skill + bonus + attr + unit_mask 全维支配才剔除），`compute_member_keep`（终章 member 用）。
- `warm_start.rs` —— 贪心选卡 + 1-swap 局部改进，产出 DFS 的初始下界（incumbent），让上界剪枝更早生效。
- `dfs.rs` —— 通用 B&B：`recurse` 按 target 分派到 `recurse_monotonic`(Power/Skill) / `recurse_score_noevent_monotonic` / `recurse_ep`(Score/Mysekai 带 event)。`TopKTracker` 维护 top-k（按 game_id 集合去重）。
- `final_chapter.rs` —— 终章专用：按 leader 角色分组 + member 两段式搜索（`recurse_chars` 选角色组合 → `recurse_cards` 选具体卡），beam seeding，per-leader 上界剪枝。
- `challenge_search.rs` —— challenge 模式：同角色多卡允许，仅按 game_id 去重。
- `types.rs` —— `DeckResult`、`DeckResultSummary`、`SearchParams`。

## 编译与依赖
- `crates/allium-deck/Cargo.toml`：**只依赖 serde / serde_json / thiserror**。无 skia/freetype/tokio。→ **可独立秒级编译**（standalone Docker 的前提，见 `docker/`）。
- 被 `allium-scapus`（根 crate）依赖：`Cargo.toml:61 allium-deck = { path = "crates/allium-deck" }`。服务侧 `OwnedGameData` 存于 `AppState.deck_game_data: Arc<ArcSwapOption<OwnedGameData>>`。
- workspace release profile（根 `Cargo.toml`）：`opt-level=3, lto="fat", codegen-units=1`。性能数字应在 release 下测。

## 测试
- 单测：散落各模块 `#[cfg(test)]`（pool/search/handler 都有）。
- e2e 回归：`tests/e2e_regression.rs` + `tests/testdata_adapter/`。读 `manifest.json` 驱动，对比 moe 的 `*_cpp_output.json`。
  - 数据路径由环境变量覆盖：`ALLIUM_TESTDATA` / `ALLIUM_MASTERDATA_CN` / `ALLIUM_MASTERDATA_JP` / `ALLIUM_MUSIC_METAS`。**standalone Docker 靠这些注入数据**。
  - `testdata/mock/`（334 个小 fixture，自带）与 `testdata/real/`（大账号 + manifest）。
  - ⚠️ `masterdata_loader.rs:222-224` 把 WL 支援表硬编码空 → WL 支援未被回归覆盖（P1）。
