//! Exhaustive result checks for pools beyond one 512-bit metadata mask.
use super::*;

fn overflow_pool() -> CardPool {
    let mut cards = vec![
        skill_card(101, 2, 1_350, 65),
        skill_card(102, 3, 1_400, 70),
        skill_card(103, 4, 1_450, 75),
    ];
    for variant in 0..509u16 {
        let mut card = skill_card(1_000 + variant, 1, 850 + u32::from(variant), 25);
        card.attr = (variant % 5) as u8;
        card.unit_mask = 1 << (variant % 6);
        card.base_bonus = (variant % 13) as u8;
        card.skill.value += (variant % 17) as u8;
        card.skill_max = card.skill.value;
        cards.push(card);
    }
    // A distinct cultivation variant appears at dense index 511. Its public
    // card ID matches index 3, so the oracle must choose one representative.
    cards[511].game_id = cards[3].game_id;
    cards[511].power += 200;
    cards[511].power_max = cards[511].power;
    cards.push(skill_card(60_000, 5, 1_500, 80));
    assert_eq!(cards.len(), 513);
    let pool = build_pool(&cards);
    assert_eq!(
        pool.card_idx(512).map(|idx| pool.game_id(idx)),
        Some(60_000)
    );
    pool
}

fn overflow_context(pool: &CardPool, scene: &str) -> SearchContext {
    let mut context = ready_ctx(pool, ScoreTarget::Score);
    context.live_type = LiveType::Multi;
    context.best_skill_as_leader = false;
    if scene != "ordinary" {
        context.is_world_bloom = true;
        context.event_type = Some(EventType::WorldBloom);
        context.live_skill_order = LiveSkillOrder::Average;
        context.support_deck = SupportDeck {
            cards: vec![(1_000, 35.0), (60_000, 30.0), (65_000, 25.0)],
            count: 2,
        };
        context.extra_bonus_ub = 100;
    }
    if scene == "final_fixed" {
        context.is_final_chapter = true;
        context.forced_leader_character_id = Some(5);
        context.fixed_card_ids = vec![60_000];
        context.support_decks_by_character = vec![context.support_deck.clone(); 27];
    }
    context
}

#[test]
fn overflow_pool_matches_independent_ordered_oracle() {
    let pool = overflow_pool();
    assert!(pool.exceeds_mask_capacity());
    let tail = pool.card_idx(512).unwrap();
    assert_eq!(pool.char_id(tail), 5);
    assert_eq!(pool.char_indices(5).collect::<Vec<_>>(), vec![tail]);

    for scene in ["ordinary", "world_bloom", "final_fixed"] {
        let context = overflow_context(&pool, scene);
        let oracle_params = SearchParams {
            top_k: 100,
            timeout_ms: 0,
        };
        let (oracle, oracle_stats) = ExactOracle::new(&pool, &context).search(&oracle_params);
        assert!(oracle_stats.evaluated > 0, "{scene}");
        assert_eq!(oracle.len(), 100, "{scene}");
        for top_k in [1, 8, 100] {
            let outcome = search(
                &pool,
                &context,
                &SearchParams {
                    top_k,
                    timeout_ms: 0,
                },
            );
            assert_eq!(
                outcome.completion(),
                SearchCompletion::Complete,
                "{scene} K={top_k}"
            );
            assert_eq!(outcome.results, oracle[..top_k], "{scene} K={top_k}");
            assert!(
                outcome
                    .results
                    .iter()
                    .all(|result| result.cards.contains(&tail))
            );
        }
    }
}
