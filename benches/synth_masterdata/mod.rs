//! 基准测试用的确定性合成 masterdata 生成器。
//!
//! 产出与真实 masterdata 目录同名、同 schema 的原始 JSON 表（`cards.json`、
//! `skills.json`、`eventDeckBonuses.json` 等），外加 music metas 与一份满配用户
//! 数据，规模贴近真实（26 角色、1300 卡、约 20 个技能原型、区域道具/剧情/突破/
//! 活动加成全套）。所有数值都是合成的，不包含任何游戏资产或真实数据，
//! `cargo bench` 对任何人开箱可跑。
//!
//! 生成结果走引擎自己的 `MasterdataSources::from_strings` →
//! `OwnedGameData::from_sources` 解析路径，schema 正确性由真实解析代码保证。

use serde_json::{json, Value};

/// 主线五团（VS 以支援团形式挂靠）。
pub const MAIN_UNITS: [&str; 5] = [
    "light_sound",
    "idol",
    "street",
    "theme_park",
    "school_refusal",
];

/// 五属性。
pub const ATTRS: [&str; 5] = ["cool", "cute", "happy", "mysterious", "pure"];

/// 角色数：1..=20 主线（每团 4 人），21..=26 VS。
pub const CHARACTER_COUNT: i32 = 26;
const VS_FIRST: i32 = 21;

/// 每角色的稀有度分布 (card_rarity_type 名, 张数)，合计 50 张 → 全局 1300 张。
const RARITY_PLAN: [(&str, usize); 5] = [
    ("rarity_1", 4),
    ("rarity_2", 6),
    ("rarity_3", 12),
    ("rarity_4", 24),
    ("rarity_birthday", 4),
];

/// (rarity 名, maxLevel, trainingMaxLevel, 每维 param 满级值, 特训固定加成/维)
const RARITY_DATA: [(&str, i32, Option<i32>, i32, i32); 5] = [
    ("rarity_1", 20, None, 1200, 0),
    ("rarity_2", 30, None, 1700, 0),
    ("rarity_3", 40, Some(50), 2600, 150),
    ("rarity_4", 50, Some(60), 3400, 300),
    ("rarity_birthday", 60, None, 2900, 0),
];

/// 生成结果：喂给 `MasterdataSources::from_strings` 的 (文件名, JSON) 列表、
/// music metas JSON、以及 camelCase 用户数据 JSON（`parse_user_profile_json` 可读）。
pub struct SynthData {
    pub tables: Vec<(String, String)>,
    pub music_metas_json: String,
    pub user_json: String,
}

/// 固定种子，保证任何机器生成逐字节一致的数据。
pub const DEFAULT_SEED: u64 = 0xA111_D3CC;

/// splitmix64：无依赖的确定性伪随机数。
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// [lo, hi] 闭区间取整。
    fn range(&mut self, lo: i64, hi: i64) -> i64 {
        debug_assert!(lo <= hi);
        lo + (self.next() % (hi - lo + 1) as u64) as i64
    }
}

/// 主生成入口。
pub fn generate(seed: u64) -> SynthData {
    let mut rng = Rng(seed);

    // ---- gameCharacterUnits：主线 20 行 + VS 每人 6 行（piapro + 五团），共 56 行 ----
    let mut gcu_rows = Vec::new();
    let mut gcu_id = 0;
    for character_id in 1..=20 {
        gcu_id += 1;
        gcu_rows.push(json!({
            "id": gcu_id,
            "gameCharacterId": character_id,
            "unit": MAIN_UNITS[((character_id - 1) / 4) as usize],
        }));
    }
    // vs_unit_row_ids[unit_index] = 该团下所有 VS 行的 gameCharacterUnitId。
    let mut vs_unit_row_ids: [Vec<i64>; 5] = Default::default();
    for character_id in VS_FIRST..=CHARACTER_COUNT {
        gcu_id += 1;
        gcu_rows.push(json!({
            "id": gcu_id,
            "gameCharacterId": character_id,
            "unit": "piapro",
        }));
        for (unit_index, unit) in MAIN_UNITS.iter().enumerate() {
            gcu_id += 1;
            vs_unit_row_ids[unit_index].push(gcu_id);
            gcu_rows.push(json!({
                "id": gcu_id,
                "gameCharacterId": character_id,
                "unit": unit,
            }));
        }
    }

    // ---- cards / cardEpisodes / 用户卡（一趟生成，episode id 全局递增） ----
    let mut card_rows = Vec::new();
    let mut episode_rows = Vec::new();
    let mut user_card_rows = Vec::new();
    // 每角色最后一张 rarity_4 的卡 ID，用于活动加成卡与默认卡组。
    let mut last_r4_by_char = vec![0i64; CHARACTER_COUNT as usize + 1];
    let mut card_id = 0i64;
    let mut episode_id = 0i64;

    for character_id in 1..=CHARACTER_COUNT {
        let is_vs = character_id >= VS_FIRST;
        for (rarity_index, (rarity, count)) in RARITY_PLAN.iter().enumerate() {
            let (_, max_level, training_max, max_param, training_bonus) = RARITY_DATA[rarity_index];
            for nth in 0..*count {
                card_id += 1;
                let attr = ATTRS[(rng.next() % 5) as usize];
                // VS 卡约 2/3 携带支援团；主线卡无 supportUnit。
                let support_unit = if is_vs && nth % 3 != 0 {
                    Some(MAIN_UNITS[(rng.next() % 5) as usize])
                } else {
                    None
                };
                let skill_id = pick_skill(&mut rng, rarity, support_unit);
                let cap = training_max.unwrap_or(max_level);
                let params = card_parameter_arrays(&mut rng, cap, max_param);
                if *rarity == "rarity_4" {
                    last_r4_by_char[character_id as usize] = card_id;
                }
                card_rows.push(json!({
                    "id": card_id,
                    "characterId": character_id,
                    "cardRarityType": rarity,
                    "attr": attr,
                    "supportUnit": support_unit.unwrap_or("none"),
                    "skillId": skill_id,
                    "specialTrainingSkillId": Value::Null,
                    "assetbundleName": format!("synth{:03}_no{:03}", character_id, card_id),
                    "specialTrainingPower1BonusFixed": training_bonus,
                    "specialTrainingPower2BonusFixed": training_bonus,
                    "specialTrainingPower3BonusFixed": training_bonus,
                    "cardParameters": params,
                }));

                // 剧情：birthday 1 篇，其余 2 篇。
                let episode_base = [100, 150, 250, 400, 250][rarity_index];
                let episode_count = if *rarity == "rarity_birthday" { 1 } else { 2 };
                let mut read_episode_ids = Vec::new();
                for part in 0..episode_count {
                    episode_id += 1;
                    let bonus = episode_base + part * episode_base / 2;
                    read_episode_ids.push(episode_id);
                    episode_rows.push(json!({
                        "id": episode_id,
                        "cardId": card_id,
                        "power1BonusFixed": bonus,
                        "power2BonusFixed": bonus,
                        "power3BonusFixed": bonus,
                    }));
                }

                // 满配用户卡：满级、满技能、满破、剧情全读、可特训的已特训。
                let trained = training_max.is_some();
                user_card_rows.push(json!({
                    "cardId": card_id,
                    "level": cap,
                    "skillLevel": 4,
                    "masterRank": 5,
                    "specialTrainingStatus": if trained { "done" } else { "not_doing" },
                    "defaultImage": if trained { "special_training" } else { "original" },
                    "episodes": read_episode_ids
                        .iter()
                        .map(|id| json!({"cardEpisodeId": id, "scenarioStatus": "already_read"}))
                        .collect::<Vec<_>>(),
                }));
            }
        }
    }
    let total_cards = card_id;

    // ---- cardRarities / masterLessons / cardMysekaiCanvasBonuses ----
    let rarity_rows = RARITY_DATA
        .iter()
        .map(
            |(rarity, max_level, training_max, _, _)| match training_max {
                Some(training) => json!({
                    "cardRarityType": rarity,
                    "maxLevel": max_level,
                    "trainingMaxLevel": training,
                    "maxSkillLevel": 4,
                }),
                None => json!({
                    "cardRarityType": rarity,
                    "maxLevel": max_level,
                    "maxSkillLevel": 4,
                }),
            },
        )
        .collect::<Vec<_>>();

    let mut master_lesson_rows = Vec::new();
    for (rarity_index, (rarity, ..)) in RARITY_DATA.iter().enumerate() {
        let per_rank = [50, 100, 150, 200, 150][rarity_index];
        for master_rank in 0..=5 {
            let bonus = per_rank * master_rank;
            master_lesson_rows.push(json!({
                "cardRarityType": rarity,
                "masterRank": master_rank,
                "power1BonusFixed": bonus,
                "power2BonusFixed": bonus,
                "power3BonusFixed": bonus,
            }));
        }
    }

    let canvas_rows = RARITY_DATA
        .iter()
        .enumerate()
        .map(|(rarity_index, (rarity, ..))| {
            let bonus = [100, 150, 200, 300, 250][rarity_index];
            json!({
                "cardRarityType": rarity,
                "power1BonusFixed": bonus,
                "power2BonusFixed": bonus,
                "power3BonusFixed": bonus,
            })
        })
        .collect::<Vec<_>>();

    // ---- skills：约 20 个共享技能原型，覆盖引擎识别的效果类型 ----
    let skill_rows = skill_archetypes();

    // ---- areaItemLevels：5 团 + 5 属性 + 26 角色道具，各 15 级 ----
    let mut area_item_rows = Vec::new();
    let mut area_item_id = 0;
    let mut push_item = |target_unit: &str,
                         target_attr: &str,
                         target_char: Option<i32>,
                         rate_step: f64,
                         all_match_step: f64,
                         rows: &mut Vec<Value>| {
        area_item_id += 1;
        for level in 1..=15 {
            let rate = round1(2.0 + rate_step * (level - 1) as f64);
            let all_match = round1(2.0 + all_match_step * (level - 1) as f64);
            rows.push(json!({
                "areaItemId": area_item_id,
                "level": level,
                "targetUnit": target_unit,
                "targetCardAttr": target_attr,
                "targetGameCharacterId": target_char,
                "power1BonusRate": rate,
                "power1AllMatchBonusRate": all_match,
                "power2BonusRate": rate,
                "power2AllMatchBonusRate": all_match,
                "power3BonusRate": rate,
                "power3AllMatchBonusRate": all_match,
            }));
        }
    };
    for unit in MAIN_UNITS {
        push_item(unit, "any", None, 0.65, 0.93, &mut area_item_rows);
    }
    for attr in ATTRS {
        push_item("any", attr, None, 0.65, 0.93, &mut area_item_rows);
    }
    for character_id in 1..=CHARACTER_COUNT {
        push_item(
            "any",
            "any",
            Some(character_id),
            0.3,
            0.3,
            &mut area_item_rows,
        );
    }
    let area_item_count = area_item_id;

    // ---- characterRanks：26 角色 × 100 级 ----
    let mut character_rank_rows = Vec::new();
    for character_id in 1..=CHARACTER_COUNT {
        for rank in 1..=100 {
            character_rank_rows.push(json!({
                "characterId": character_id,
                "characterRank": rank,
                "power1BonusRate": round1(f64::from(rank) * 0.1),
            }));
        }
    }

    // ---- events：1 = marathon（light_sound × cool），2 = cheerful_carnival（street × happy） ----
    let event_rows = vec![
        json!({"id": 1, "eventType": "marathon"}),
        json!({"id": 2, "eventType": "cheerful_carnival"}),
    ];

    let mut event_deck_bonus_rows = Vec::new();
    let mut edb_id = 0;
    for (event_id, unit_index, attr) in [(1, 0usize, "cool"), (2, 2usize, "happy")] {
        // 该团的 gameCharacterUnitId：主线 4 人 + 所有挂靠该团的 VS 行。
        let mut unit_row_ids: Vec<i64> = (1..=4).map(|n| (unit_index as i64) * 4 + n).collect();
        unit_row_ids.extend(&vs_unit_row_ids[unit_index]);
        for row_id in &unit_row_ids {
            edb_id += 1;
            event_deck_bonus_rows.push(json!({
                "id": edb_id, "eventId": event_id,
                "gameCharacterUnitId": row_id, "cardAttr": attr, "bonusRate": 50.0,
            }));
            edb_id += 1;
            event_deck_bonus_rows.push(json!({
                "id": edb_id, "eventId": event_id,
                "gameCharacterUnitId": row_id, "cardAttr": Value::Null, "bonusRate": 20.0,
            }));
        }
        edb_id += 1;
        event_deck_bonus_rows.push(json!({
            "id": edb_id, "eventId": event_id,
            "gameCharacterUnitId": Value::Null, "cardAttr": attr, "bonusRate": 25.0,
        }));
    }

    // 活动卡：当期团 4 名主线角色 + 2 名 VS 的最新 rarity_4，各 +20%。
    let mut event_card_rows = Vec::new();
    for (event_id, unit_index) in [(1, 0usize), (2, 2usize)] {
        let mut member_chars: Vec<i32> = (1..=4).map(|n| unit_index as i32 * 4 + n).collect();
        member_chars.push(VS_FIRST);
        member_chars.push(VS_FIRST + 1);
        for character_id in member_chars {
            event_card_rows.push(json!({
                "eventId": event_id,
                "cardId": last_r4_by_char[character_id as usize],
                "bonusRate": 20.0,
                "leaderBonusRate": 0.0,
            }));
        }
    }

    // 稀有度 × master rank 活动加成（引擎会按活动 ID 展开）。
    let mut event_rarity_rows = Vec::new();
    for (rarity_index, (rarity, ..)) in RARITY_DATA.iter().enumerate() {
        for master_rank in 0..=5 {
            let rate = match rarity_index {
                3 => 25.0 + f64::from(master_rank),      // rarity_4
                4 => 5.0 + 2.0 * f64::from(master_rank), // birthday
                2 => f64::from(master_rank),             // rarity_3
                _ => 0.0,
            };
            event_rarity_rows.push(json!({
                "cardRarityType": rarity,
                "masterRank": master_rank,
                "bonusRate": rate,
            }));
        }
    }

    let world_bloom_diff_attr_rows: Vec<Value> = [0.0, 10.0, 20.0, 35.0, 50.0]
        .iter()
        .enumerate()
        .map(|(index, rate)| json!({"attributeCount": index + 1, "bonusRate": rate}))
        .collect();

    // ---- music metas：12 首 × 6 难度，含引擎合成 omakase(10000) 所需的行 ----
    let music_metas = music_meta_rows(&mut rng);

    // ---- 用户数据（camelCase，走 parse_user_profile_json） ----
    let user_characters: Vec<Value> = (1..=CHARACTER_COUNT)
        .map(|id| json!({"characterId": id, "characterRank": 100}))
        .collect();
    let user_area_items: Vec<Value> = (1..=area_item_count)
        .map(|id| json!({"areaItemId": id, "level": 15}))
        .collect();
    let deck_members: Vec<i64> = (1..=5)
        .map(|character_id| last_r4_by_char[character_id as usize])
        .collect();
    let user = json!({
        "userCards": user_card_rows,
        "userCharacters": user_characters,
        "userAreas": [{"areaId": 1, "areaItems": user_area_items}],
        "userDecks": [{
            "deckId": 1,
            "member1": deck_members[0], "member2": deck_members[1],
            "member3": deck_members[2], "member4": deck_members[3],
            "member5": deck_members[4],
        }],
        "userWorldBloomSupportDecks": [],
        "userChallengeLiveSoloDecks": [],
        "userMysekaiFixtureGameCharacterPerformanceBonuses": [],
        "userMysekaiGates": [],
        // 全卡画布加成，贴近满配账号。
        "userMysekaiCanvases": (1..=total_cards)
            .map(|id| json!({"cardId": id}))
            .collect::<Vec<_>>(),
        "userHonors": [],
    });

    let tables = vec![
        table("gameCharacterUnits.json", &gcu_rows),
        table("cards.json", &card_rows),
        table("cardRarities.json", &rarity_rows),
        table("cardEpisodes.json", &episode_rows),
        table("masterLessons.json", &master_lesson_rows),
        table("skills.json", &skill_rows),
        table("areaItemLevels.json", &area_item_rows),
        table("characterRanks.json", &character_rank_rows),
        table("cardMysekaiCanvasBonuses.json", &canvas_rows),
        table("events.json", &event_rows),
        table("eventCards.json", &event_card_rows),
        table("eventDeckBonuses.json", &event_deck_bonus_rows),
        table("eventCardBonusLimits.json", &[]),
        table("eventHonorBonuses.json", &[]),
        table("eventSkillScoreUpLimits.json", &[]),
        table("eventRarityBonusRates.json", &event_rarity_rows),
        table(
            "worldBloomDifferentAttributeBonuses.json",
            &world_bloom_diff_attr_rows,
        ),
    ];

    SynthData {
        tables,
        music_metas_json: serde_json::to_string(&music_metas).expect("serialize music metas"),
        user_json: serde_json::to_string(&user).expect("serialize user"),
    }
}

fn table(name: &str, rows: &[Value]) -> (String, String) {
    (
        name.to_string(),
        serde_json::to_string(rows).expect("serialize table"),
    )
}

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

/// 三维成长曲线：level 1 起点约为满级值的 40%，线性升至满级值，带 ±3% 抖动。
fn card_parameter_arrays(rng: &mut Rng, cap: i32, max_param: i32) -> Value {
    let mut arrays: [Vec<i64>; 3] = Default::default();
    for array in &mut arrays {
        let jitter = rng.range(-30, 30); // 每维独立 ±3% 抖动
        let max = i64::from(max_param) * (1000 + jitter) / 1000;
        let base = max * 2 / 5;
        for level in 1..=i64::from(cap) {
            array.push(base + (max - base) * (level - 1) / (i64::from(cap) - 1));
        }
    }
    json!({"param1": arrays[0], "param2": arrays[1], "param3": arrays[2]})
}

/// 按稀有度挑技能：VS 带支援团的 rarity_4 用对应团的强化技能（id 6..=10）。
fn pick_skill(rng: &mut Rng, rarity: &str, support_unit: Option<&str>) -> i64 {
    match rarity {
        "rarity_1" => 1,
        "rarity_2" => [1, 2][(rng.next() % 2) as usize],
        "rarity_3" => [2, 3][(rng.next() % 2) as usize],
        "rarity_birthday" => [3, 5][(rng.next() % 2) as usize],
        _ => {
            if let Some(unit) = support_unit {
                let unit_index = MAIN_UNITS.iter().position(|u| *u == unit).unwrap_or(0);
                return 6 + unit_index as i64;
            }
            [4, 11, 12, 13, 14][(rng.next() % 5) as usize]
        }
    }
}

/// 技能等级 1..=4 的 detail 行。
fn levels(values: [i64; 4]) -> Vec<Value> {
    values
        .iter()
        .enumerate()
        .map(|(index, value)| json!({"level": index + 1, "activateEffectValue": value}))
        .collect()
}

/// 技能原型（id 固定，供 `pick_skill` 引用）：
/// 1/2 普通加分，3 加分+回血，4 血量条件加分，5 keep 加分，
/// 6..=10 五团同队强化加分（VS 卡），11 高倍率加分，
/// 12 加分+角色等级加成，13 加分+队友技能参照，14 加分+异团计数。
fn skill_archetypes() -> Vec<Value> {
    let mut rows = Vec::new();
    let score_up = |id: i64, values: [i64; 4]| -> Value {
        json!({"id": id, "skillEffects": [
            {"skillEffectType": "score_up", "skillEffectDetails": levels(values)},
        ]})
    };
    rows.push(score_up(1, [20, 25, 30, 40]));
    rows.push(score_up(2, [40, 45, 50, 60]));
    rows.push(json!({"id": 3, "skillEffects": [
        {"skillEffectType": "score_up", "skillEffectDetails": levels([60, 65, 70, 80])},
        {"skillEffectType": "life_recovery", "skillEffectDetails": levels([250, 300, 350, 450])},
    ]}));
    rows.push(json!({"id": 4, "skillEffects": [
        {"skillEffectType": "score_up_condition_life",
         "skillEffectDetails": levels([100, 105, 110, 120])},
    ]}));
    rows.push(json!({"id": 5, "skillEffects": [
        {"skillEffectType": "score_up_keep", "skillEffectDetails": levels([80, 85, 90, 100])},
    ]}));
    for (unit_index, unit) in MAIN_UNITS.iter().enumerate() {
        rows.push(json!({"id": 6 + unit_index, "skillEffects": [
            {"skillEffectType": "score_up",
             "skillEnhance": {
                 "activateEffectValue": 10,
                 "skillEnhanceCondition": {"unit": unit},
             },
             "skillEffectDetails": levels([70, 75, 80, 90])},
        ]}));
    }
    rows.push(score_up(11, [90, 95, 100, 110]));
    let rank_effects: Vec<Value> = [(20, 5), (40, 8), (60, 10)]
        .iter()
        .map(|(rank, bonus)| {
            json!({"skillEffectType": "score_up_character_rank",
                   "activateCharacterRank": rank,
                   "skillEffectDetails": levels([*bonus; 4])})
        })
        .collect();
    let mut skill12_effects = vec![json!({
        "skillEffectType": "score_up",
        "skillEffectDetails": levels([70, 75, 80, 90]),
    })];
    skill12_effects.extend(rank_effects);
    rows.push(json!({"id": 12, "skillEffects": skill12_effects}));
    rows.push(json!({"id": 13, "skillEffects": [
        {"skillEffectType": "score_up", "skillEffectDetails": levels([100, 105, 110, 120])},
        {"skillEffectType": "other_member_score_up_reference_rate",
         "skillEffectDetails": [
             {"level": 1, "activateEffectValue": 25, "activateEffectValue2": 130},
             {"level": 2, "activateEffectValue": 30, "activateEffectValue2": 130},
             {"level": 3, "activateEffectValue": 35, "activateEffectValue2": 130},
             {"level": 4, "activateEffectValue": 40, "activateEffectValue2": 130},
         ]},
    ]}));
    rows.push(json!({"id": 14, "skillEffects": [
        {"skillEffectType": "score_up", "skillEffectDetails": levels([100, 105, 110, 120])},
        {"skillEffectType": "score_up_unit_count", "activateUnitCount": 1,
         "skillEffectDetails": levels([110, 115, 120, 130])},
        {"skillEffectType": "score_up_unit_count", "activateUnitCount": 2,
         "skillEffectDetails": levels([120, 125, 130, 140])},
    ]}));
    rows
}

/// music metas：12 首 × easy..append 六难度（snake_case 行，与真实文件同构）。
fn music_meta_rows(rng: &mut Rng) -> Vec<Value> {
    const DIFFS: [(&str, f64, i64); 6] = [
        ("easy", 1.00, 90),
        ("normal", 1.05, 220),
        ("hard", 1.12, 420),
        ("expert", 1.20, 700),
        ("master", 1.30, 950),
        ("append", 1.35, 1050),
    ];
    let mut rows = Vec::new();
    for music_id in 1..=12 {
        let music_time = rng.range(100, 145) as f64;
        let event_rate = rng.range(100, 135);
        let song_base = 0.95 + rng.range(0, 20) as f64 / 100.0;
        for (difficulty, factor, taps) in DIFFS {
            let base_score = song_base * factor;
            let mut solo = [0.0; 6];
            let mut auto = [0.0; 6];
            let mut multi = [0.0; 6];
            for slot in 0..6 {
                let s = 0.028 + rng.range(0, 30) as f64 / 1000.0;
                solo[slot] = s;
                auto[slot] = s * 0.7;
                multi[slot] = if slot == 4 { s * 1.5 } else { s };
            }
            rows.push(json!({
                "music_id": music_id,
                "difficulty": difficulty,
                "music_time": music_time,
                "event_rate": event_rate,
                "base_score": base_score,
                "base_score_auto": base_score * 0.7,
                "skill_score_solo": solo,
                "skill_score_auto": auto,
                "skill_score_multi": multi,
                "fever_score": base_score * 0.2,
                "tap_count": taps + rng.range(0, 60),
            }));
        }
    }
    rows
}
