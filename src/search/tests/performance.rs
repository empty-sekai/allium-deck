//! performance contracts.
use super::*;

#[test]
#[ignore = "controlled benchmark; compare complete results on one pinned CPU"]
fn benchmark_final_group_plans() {
    use std::time::Instant;

    let pool = build_pool(&longtail_cards(22, 6));
    let mut ctx = final_chapter_ctx(&pool);
    ctx.is_world_bloom = true;
    ctx.event_type = Some(EventType::WorldBloom);
    ctx.skill_scores[1] = [0.21, 0.17, 0.13, 0.11, 0.07, 0.23];
    ctx.diff_attr_bonus = [0, 0, 4, 24, 120, 800];
    ctx.support_decks_by_character = (0..27)
        .map(|character| longtail_support(&pool, character))
        .collect();
    for fixed in [false, true] {
        ctx.forced_leader_character_id = fixed.then_some(1);
        for top_k in [1, 8, 100] {
            let params = SearchParams {
                top_k,
                timeout_ms: 30_000,
            };
            let mut times = Vec::new();
            let mut reference = None;
            for _ in 0..5 {
                let start = Instant::now();
                let result = search(&pool, &ctx, &params);
                times.push(start.elapsed().as_secs_f64() * 1000.0);
                assert_eq!(result.completion(), SearchCompletion::Complete);
                if let Some(previous) = &reference {
                    assert_eq!(&result.results, previous);
                }
                reference = Some(result.results);
            }
            let results = reference.unwrap();
            eprintln!(
                "FINAL_PLANS fixed={fixed} top_k={top_k} median_ms={:.6} results={results:?}",
                median_f64(&mut times)
            );
        }
    }
}

#[test]
#[ignore = "controlled benchmark; run single-threaded on one pinned CPU"]
fn benchmark_wl_and_final_exact_bounds() {
    use std::time::Instant;

    let cards = longtail_cards(22, 6);
    let pool = build_pool(&cards);
    let search_params = SearchParams {
        top_k: 8,
        timeout_ms: 300_000,
    };

    let mut wl_ctx = ready_ctx(&pool, ScoreTarget::Score);
    wl_ctx.live_type = LiveType::Multi;
    wl_ctx.event_type = Some(EventType::WorldBloom);
    wl_ctx.is_world_bloom = true;
    wl_ctx.skill_scores[1] = [0.21, 0.17, 0.13, 0.11, 0.07, 0.23];
    wl_ctx.diff_attr_bonus = [0, 0, 4, 24, 120, 800];
    wl_ctx.support_deck = longtail_support(&pool, 0);

    let mut final_ctx = final_chapter_ctx(&pool);
    final_ctx.is_world_bloom = true;
    final_ctx.event_type = Some(EventType::WorldBloom);
    final_ctx.skill_scores[1] = [0.21, 0.17, 0.13, 0.11, 0.07, 0.23];
    final_ctx.diff_attr_bonus = [0, 0, 4, 24, 120, 800];
    final_ctx.support_decks_by_character = vec![SupportDeck::default(); 27];
    for char_id in 1usize..=22 {
        final_ctx.support_decks_by_character[char_id] = longtail_support(&pool, char_id);
    }

    fn bench(
        pool: &CardPool,
        ctx: &SearchContext,
        params: &SearchParams,
        final_bound: bool,
        disabled: bool,
        repeats: usize,
    ) -> (f64, Vec<DeckResult>, SearchStats) {
        let mut tuning = tuning::SearchTuning::default();
        if final_bound {
            tuning.final_attr_dp = !disabled;
        } else {
            tuning.world_bloom_attr_matching = !disabled;
        }
        let mut times = Vec::with_capacity(repeats);
        let mut last = None;
        for _ in 0..repeats {
            let started = Instant::now();
            let result = tuning::with_tuning(tuning, || search_instrumented(pool, ctx, params));
            times.push(started.elapsed().as_secs_f64() * 1000.0);
            last = Some(result);
        }
        let p50 = median_f64(&mut times);
        let (results, stats) = last.unwrap();
        (p50, results, stats)
    }

    let (wl_old_ms, wl_old, wl_old_stats) = bench(&pool, &wl_ctx, &search_params, false, true, 7);
    let (wl_new_ms, wl_new, wl_new_stats) = bench(&pool, &wl_ctx, &search_params, false, false, 7);
    assert_property_results(&pool, &wl_new, &wl_old, "WL bound A/B");

    let (final_old_ms, final_old, final_old_stats) =
        bench(&pool, &final_ctx, &search_params, true, true, 5);
    let (final_new_ms, final_new, final_new_stats) =
        bench(&pool, &final_ctx, &search_params, true, false, 5);
    assert_property_results(&pool, &final_new, &final_old, "Final attr DP A/B");

    eprintln!(
        "WL_AB old_ms={wl_old_ms:.3} new_ms={wl_new_ms:.3} old={wl_old_stats:?} new={wl_new_stats:?}"
    );
    eprintln!(
        "FINAL_AB old_ms={final_old_ms:.3} new_ms={final_new_ms:.3} old={final_old_stats:?} new={final_new_stats:?}"
    );
}

#[test]
#[ignore = "controlled benchmark; run single-threaded on one pinned CPU"]
fn benchmark_power_skill_exact_longtail() {
    use std::time::Instant;

    let cards = longtail_cards(22, 6);
    let pool = build_pool(&cards);
    let params = SearchParams {
        top_k: 8,
        timeout_ms: 30_000,
    };

    fn run(
        pool: &CardPool,
        ctx: &SearchContext,
        params: &SearchParams,
        repeats: usize,
    ) -> (f64, Vec<DeckResult>, SearchStats) {
        let mut times = Vec::with_capacity(repeats);
        let mut last = None;
        for _ in 0..repeats {
            let started = Instant::now();
            let result = search_instrumented(pool, ctx, params);
            times.push(started.elapsed().as_secs_f64() * 1000.0);
            last = Some(result);
        }
        let p50 = median_f64(&mut times);
        let (results, stats) = last.unwrap();
        (p50, results, stats)
    }

    let mut power_ctx = ready_ctx(&pool, ScoreTarget::Power);
    power_ctx.fixed_card_ids = vec![pool.game_id(CardIdx::new(0))];
    power_ctx.fixed_character_ids = vec![2];
    let (power_ms, _, power_stats) = run(&pool, &power_ctx, &params, 3);

    let mut power_min_ctx = power_ctx.clone();
    power_min_ctx.minimize = true;
    let (power_min_ms, _, power_min_stats) = run(&pool, &power_min_ctx, &params, 3);

    let mut skill_ctx = ready_ctx(&pool, ScoreTarget::Skill);
    skill_ctx.fixed_character_ids = vec![1];
    let (skill_ms, _, skill_stats) = run(&pool, &skill_ctx, &params, 3);

    eprintln!(
        "SIMPLE_EXACT power_ms={power_ms:.3} power={power_stats:?} power_min_ms={power_min_ms:.3} power_min={power_min_stats:?} skill_ms={skill_ms:.3} skill={skill_stats:?}"
    );

    assert!(
        !power_stats.deadline_hit,
        "constrained power benchmark timed out"
    );
    assert!(
        !power_min_stats.deadline_hit,
        "power minimize benchmark timed out"
    );
    assert!(!skill_stats.deadline_hit, "skill benchmark timed out");
}
