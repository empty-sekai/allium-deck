//! Reproducible search-only samples. Inputs are synthetic, never traffic estimates.
use super::*;
use std::io::Write;

#[test]
#[ignore = "controlled matrix; set ALLIUM_VALIDATION_OUT and pin one CPU"]
fn validation_performance_matrix() {
    let path = std::env::var("ALLIUM_VALIDATION_OUT").expect("ALLIUM_VALIDATION_OUT is required");
    let mut output = std::io::BufWriter::new(
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .unwrap(),
    );
    let numbers = |name: &str, default: &str| -> Vec<usize> {
        std::env::var(name)
            .unwrap_or_else(|_| default.into())
            .split(',')
            .map(|value| value.parse().unwrap())
            .collect()
    };
    let sizes = numbers("ALLIUM_VALIDATION_SIZES", "26,78,132");
    let topks = numbers("ALLIUM_VALIDATION_TOPKS", "1,8,30,100");
    let seeds = numbers("ALLIUM_VALIDATION_SEEDS", "0,1,2,3");
    let repeats = numbers("ALLIUM_VALIDATION_REPEATS", "1")[0];
    let timeout_ms = numbers("ALLIUM_VALIDATION_TIMEOUT_MS", "0")[0] as u64;
    let start_case = numbers("ALLIUM_VALIDATION_CASE_START", "0")[0];
    let limit = numbers("ALLIUM_VALIDATION_CASE_LIMIT", "999999")[0];
    let mut case = 0;
    for family in ["random", "scarce_attributes"] {
        for &size in &sizes {
            assert!((5..=crate::pool::MASK_WORDS * 64).contains(&size));
            for &seed in &seeds {
                let cards = matrix_cards(family, size, seed);
                let pool = build_pool(&cards);
                for scene in ["world_bloom", "final_fixed", "final_auto"] {
                    let ctx = matrix_context(&pool, scene, seed);
                    let input_checksum = matrix_input_checksum(&cards, &ctx);
                    for &top_k in &topks {
                        let case_id = case;
                        case += 1;
                        if case_id < start_case || case_id - start_case >= limit {
                            continue;
                        }
                        let params = SearchParams { top_k, timeout_ms };
                        let mut reference = None;
                        // Warmup is retained as a labeled row and excluded by the reporter.
                        for sample in 0..=repeats {
                            let started = std::time::Instant::now();
                            let outcome = search(&pool, &ctx, &params);
                            let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
                            if outcome.completion() == SearchCompletion::Complete {
                                if let Some(previous) = &reference {
                                    assert_eq!(&outcome.results, previous, "case {case_id}");
                                }
                                reference = Some(outcome.results.clone());
                            }
                            let results: Vec<_> = outcome.results.iter().map(|result| {
                                assert_eq!(evaluate::leaf_evaluate_checked(&pool, &ctx, &result.cards), Some(result.score));
                                serde_json::json!({
                                    "score": result.score.to_string(),
                                    "ordered_cards": result.cards.map(|card| pool.game_id(card)),
                                    "card_set": result.game_card_set_key(&pool),
                                })
                            }).collect();
                            serde_json::to_writer(
                                &mut output,
                                &serde_json::json!({
                                    "schema": 1, "source": "synthetic", "case": case_id,
                                    "family": family, "seed": seed, "scene": scene,
                                "pool_size": pool.count(), "top_k": top_k,
                                "input_checksum_fnv1a64": input_checksum,
                                    "sample": sample, "warmup": sample == 0,
                                    "timeout_ms": timeout_ms, "elapsed_ms": elapsed_ms,
                                    "completion": outcome.completion(), "stats": outcome.stats,
                                    "results": results,
                                }),
                            )
                            .unwrap();
                            writeln!(output).unwrap();
                            output.flush().unwrap();
                        }
                    }
                }
            }
        }
    }
}

// Reproducibility check only; the runner separately records cryptographic
// source, executable and artifact hashes.
fn matrix_input_checksum(cards: &[TestCard], ctx: &SearchContext) -> String {
    let rows: Vec<_> = cards
        .iter()
        .map(|card| {
            (
                card.char_id,
                card.attr,
                card.unit_mask,
                card.game_id,
                card.power,
                card.skill.skill_type,
                card.skill.value,
                card.base_bonus,
                card.limited_bonus,
                card.power_max,
                card.skill_max,
            )
        })
        .collect();
    let input = format!("{}|{ctx:?}", serde_json::to_string(&rows).unwrap());
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in input.bytes() {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

fn matrix_cards(family: &str, size: usize, seed: usize) -> Vec<TestCard> {
    let mut cards = randomized_exact_cards(0xACED_0000 + seed as u64, size, 26);
    if family == "scarce_attributes" {
        for (index, card) in cards.iter_mut().enumerate() {
            let variant = index / 26;
            card.attr = if variant == 2 && card.char_id <= 3 {
                card.char_id + 1
            } else {
                (variant % 2) as u8
            };
            let tradeoff = variant % 20;
            card.power = 6000 - tradeoff as u32 * 180 + u32::from(card.char_id) * 7;
            card.power_max = card.power;
            card.skill.value = 30 + tradeoff as u8 * 10;
            card.skill_max = card.skill.value;
            card.base_bonus = 8 + tradeoff as u8 * 5;
            card.limited_bonus = if variant >= 3 { 5 } else { 0 };
        }
    }
    cards
}

fn matrix_context(pool: &CardPool, scene: &str, seed: usize) -> SearchContext {
    let mut ctx = if scene == "world_bloom" {
        ready_ctx(pool, ScoreTarget::Score)
    } else {
        final_chapter_ctx(pool)
    };
    ctx.live_type = LiveType::Multi;
    ctx.is_world_bloom = true;
    ctx.event_type = Some(EventType::WorldBloom);
    ctx.skill_scores[1] = [0.21, 0.17, 0.13, 0.11, 0.07, 0.23];
    ctx.diff_attr_bonus = if seed.is_multiple_of(2) {
        [0, 0, 4, 24, 120, 800]
    } else {
        [0, 0, 18, 81, 29, 117]
    };
    ctx.support_deck = longtail_support(pool, seed);
    ctx.support_decks_by_character = (0..27)
        .map(|character| longtail_support(pool, seed + character))
        .collect();
    if scene == "final_fixed" {
        ctx.forced_leader_character_id = Some(1);
    }
    for dense in 0..pool.count() {
        ctx.leader_honor_bonus_x10[dense] = ((dense * 3 + seed) % 9) as u16 * 10;
        ctx.leader_limit_bonus_x10[dense] = ((dense * 5 + seed) % 7) as u16 * 10;
    }
    ctx
}
