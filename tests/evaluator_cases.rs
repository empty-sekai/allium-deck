use allium_deck::eval::evaluate;
use allium_deck::*;
use std::fs;
use std::path::Path;

fn pd(total: i32) -> PowerDetail {
    PowerDetail {
        base: total,
        total,
        ..PowerDetail::default()
    }
}

fn skill(score_up: f64) -> SkillInfo {
    SkillInfo {
        skill_id: score_up as i32 + 1,
        base_score_up: score_up,
        ..SkillInfo::default()
    }
}

fn ref_skill(base: f64, rate: f64, max: f64) -> SkillInfo {
    SkillInfo {
        skill_id: 900,
        base_score_up: base,
        has_ref: true,
        ref_rate: rate,
        ref_max: max,
        ..SkillInfo::default()
    }
}

fn lookup_power(total: i32) -> PowerLookup {
    let detail = pd(total);
    PowerLookup {
        resolved: [[detail; 4]; UNIT_COUNT],
        diff: [detail; 3],
    }
}

fn lookup_skill(info: SkillInfo) -> SkillLookup {
    SkillLookup {
        resolved: [[info; 2]; UNIT_COUNT],
        diff: [info; 3],
    }
}

fn card(
    id: CardId,
    character_id: i32,
    attr: Attr,
    unit: Unit,
    total_power: i32,
    score_up: f64,
    event_bonus: CardEventBonus,
) -> CardSpec {
    CardSpec {
        card_id: id,
        character_id,
        attr,
        support_unit: Unit::None,
        units: [unit, Unit::None, Unit::None],
        unit_count: 1,
        power: lookup_power(total_power),
        skill: lookup_skill(skill(score_up)),
        event_bonus,
        default_image: DefaultImage::SpecialTraining,
    }
}

fn pool(mut cards: Vec<CardSpec>) -> CardPool {
    cards.sort_by_key(|card| card.card_id);
    let count = cards.len();
    let character_ids = cards.iter().map(|card| card.character_id).collect();
    let attrs = cards.iter().map(|card| card.attr).collect();
    let support_units = cards.iter().map(|card| card.support_unit).collect();
    let game_ids = cards.iter().map(|card| card.card_id as i32).collect();
    CardPool {
        cards,
        character_ids,
        attrs,
        support_units,
        game_ids,
        count: count as u16,
    }
}

fn base_cards(power: i32, skill: f64) -> Vec<CardSpec> {
    vec![
        card(
            0,
            1,
            Attr::Cool,
            Unit::LightSound,
            power,
            skill,
            CardEventBonus::default(),
        ),
        card(
            1,
            2,
            Attr::Cute,
            Unit::Idol,
            power,
            skill,
            CardEventBonus::default(),
        ),
        card(
            2,
            3,
            Attr::Happy,
            Unit::Street,
            power,
            skill,
            CardEventBonus::default(),
        ),
        card(
            3,
            4,
            Attr::Pure,
            Unit::Themepark,
            power,
            skill,
            CardEventBonus::default(),
        ),
        card(
            4,
            5,
            Attr::Mysterious,
            Unit::SchoolRefusal,
            power,
            skill,
            CardEventBonus::default(),
        ),
    ]
}

fn music() -> MusicParams {
    MusicParams {
        event_rate: 100.0,
        base_score: 1.0,
        base_score_auto: 1.0,
        fever_score: 0.0,
        skill_scores: [[0.0; 6]; 3],
        music_time: 100.0,
        tap_count: 100,
    }
}

fn params(
    event: Option<EventContext>,
    live_type: LiveType,
    target: ScoreTarget,
) -> DeckContextParams {
    DeckContextParams {
        honor_bonus: 0,
        music: music(),
        live_type,
        target,
        event,
        skill_reference_strategy: SkillReferenceStrategy::Average,
        keep_after_training_state: false,
        best_skill_as_leader: true,
        live_skill_order: LiveSkillOrder::Average,
        specific_skill_order: None,
        multi_teammate_score_up: None,
        multi_teammate_power: None,
    }
}

fn event(event_type: EventType) -> EventContext {
    EventContext {
        event_id: 1,
        event_type,
        boost_rate: 1.0,
        other_score: None,
        life: 1000,
        custom_bonus: None,
        world_bloom: None,
        skill_score_up_limit: None,
        card_bonus_count_limit: DECK_SIZE,
    }
}

fn skill_order_music() -> MusicParams {
    MusicParams {
        event_rate: 100.0,
        base_score: 1.0,
        base_score_auto: 1.0,
        fever_score: 0.0,
        skill_scores: [[1.0, 2.0, 3.0, 4.0, 5.0, 0.0], [0.0; 6], [0.0; 6]],
        music_time: 100.0,
        tap_count: 100,
    }
}

fn skill_order_params(order: LiveSkillOrder) -> DeckContextParams {
    DeckContextParams {
        music: skill_order_music(),
        live_skill_order: order,
        best_skill_as_leader: false,
        ..params(None, LiveType::Solo, ScoreTarget::Score)
    }
}

fn skill_order_pool() -> CardPool {
    pool(vec![
        card(
            0,
            1,
            Attr::Cool,
            Unit::LightSound,
            1000,
            10.0,
            CardEventBonus::default(),
        ),
        card(
            1,
            2,
            Attr::Cute,
            Unit::Idol,
            1000,
            20.0,
            CardEventBonus::default(),
        ),
        card(
            2,
            3,
            Attr::Happy,
            Unit::Street,
            1000,
            30.0,
            CardEventBonus::default(),
        ),
        card(
            3,
            4,
            Attr::Pure,
            Unit::Themepark,
            1000,
            40.0,
            CardEventBonus::default(),
        ),
        card(
            4,
            5,
            Attr::Mysterious,
            Unit::SchoolRefusal,
            1000,
            50.0,
            CardEventBonus::default(),
        ),
    ])
}

fn output(case: &str, score: DeckScore) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/eval");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join(format!("{case}_output.json")),
        serde_json::to_string_pretty(&score).unwrap(),
    )
    .unwrap();
}

#[test]
fn score_multi_ev() {
    let pool = pool(base_cards(1000, 0.0));
    let ctx = DeckContext::new(
        &pool,
        params(
            Some(event(EventType::Marathon)),
            LiveType::Multi,
            ScoreTarget::Score,
        ),
    )
    .unwrap();
    let score = evaluate(&[0, 1, 2, 3, 4], &ctx);
    output("score_multi_ev", score);
    assert_eq!(score.total_power, 5000);
    assert_eq!(score.live_score, 21875);
    assert_eq!(score.event_point, 111);

    let cheerful_ctx = DeckContext::new(
        &pool,
        params(
            Some(event(EventType::CheerfulCarnival)),
            LiveType::Multi,
            ScoreTarget::Score,
        ),
    )
    .unwrap();
    let cheerful = evaluate(&[0, 1, 2, 3, 4], &cheerful_ctx);
    assert_eq!(cheerful.live_score, 20000);
}

#[test]
fn wl_bonus() {
    let pool = pool(
        base_cards(1000, 0.0)
            .into_iter()
            .map(|mut card| {
                card.event_bonus.base_bonus = 10.0;
                card
            })
            .collect(),
    );
    let mut table = [0.0; ATTR_COUNT];
    table[1] = 0.0;
    table[2] = 1.0;
    table[3] = 2.0;
    table[4] = 3.0;
    table[5] = 4.0;
    let support_cards = (10..40)
        .map(|card_id| SupportDeckCard {
            card_id,
            bonus: 1.0,
        })
        .collect();
    let event = EventContext {
        event_type: EventType::WorldBloom,
        world_bloom: Some(WorldBloomContext {
            support_deck_count: 12,
            diff_attr_bonus_table: table,
            support_cards,
            final_chapter_support: Vec::new(),
            power_total_cap: None,
        }),
        ..event(EventType::WorldBloom)
    };
    let ctx = DeckContext::new(
        &pool,
        params(Some(event), LiveType::Solo, ScoreTarget::Bonus),
    )
    .unwrap();
    let score = evaluate(&[0, 1, 2, 3, 4], &ctx);
    output("wl_bonus", score);
    assert_eq!(score.diff_attr_bonus_rate, 4.0);
    assert_eq!(score.event_bonus_rate, 54.0);
    assert_eq!(score.support_deck_bonus_rate, 12.0);
}

#[test]
fn wl3() {
    let pool = pool(base_cards(100000, 0.0));
    let support_cards = (10..40)
        .map(|card_id| SupportDeckCard {
            card_id,
            bonus: 1.0,
        })
        .collect();
    let event = EventContext {
        event_type: EventType::WorldBloom,
        world_bloom: Some(WorldBloomContext {
            support_deck_count: 25,
            diff_attr_bonus_table: [0.0; ATTR_COUNT],
            support_cards,
            final_chapter_support: Vec::new(),
            power_total_cap: Some(336000),
        }),
        ..event(EventType::WorldBloom)
    };
    let ctx = DeckContext::new(
        &pool,
        params(Some(event), LiveType::Solo, ScoreTarget::Power),
    )
    .unwrap();
    let score = evaluate(&[0, 1, 2, 3, 4], &ctx);
    output("wl3", score);
    assert_eq!(score.total_power, 336000);
    assert_eq!(score.support_deck_bonus_rate, 25.0);
}

#[test]
fn wl3_finale() {
    let mut cards = base_cards(1000, 0.0);
    for card in &mut cards {
        card.event_bonus.limited_bonus = 10.0;
        card.event_bonus.leader_honor_bonus = 2.0;
        card.event_bonus.leader_limit_bonus = 3.0;
    }
    cards[4].skill = lookup_skill(skill(500.0));
    let pool = pool(cards);
    let event = EventContext {
        event_id: FINAL_CHAPTER_EVENT_ID,
        event_type: EventType::WorldBloom,
        world_bloom: Some(WorldBloomContext {
            support_deck_count: 25,
            diff_attr_bonus_table: [0.0; ATTR_COUNT],
            support_cards: Vec::new(),
            final_chapter_support: Vec::new(),
            power_total_cap: None,
        }),
        card_bonus_count_limit: 4,
        ..event(EventType::WorldBloom)
    };
    let ctx = DeckContext::new(
        &pool,
        params(Some(event), LiveType::Solo, ScoreTarget::Bonus),
    )
    .unwrap();
    let score = evaluate(&[0, 1, 2, 3, 4], &ctx);
    output("wl3_finale", score);
    assert_eq!(score.event_bonus_rate, 45.0);
    assert_eq!(score.card_ids[0], 0);
}

#[test]
fn skill_limit() {
    let pool = pool(base_cards(1000, 400.0));
    let limited_event = EventContext {
        skill_score_up_limit: Some(300.0),
        ..event(EventType::Marathon)
    };
    let ctx = DeckContext::new(
        &pool,
        params(Some(limited_event), LiveType::Solo, ScoreTarget::Skill),
    )
    .unwrap();
    let score = evaluate(&[0, 1, 2, 3, 4], &ctx);
    output("skill_limit", score);
    assert_eq!(score.card_skill_score_up[0], 300.0);

    let challenge_ctx = DeckContext::new(
        &pool,
        params(
            Some(event(EventType::Marathon)),
            LiveType::Challenge,
            ScoreTarget::Skill,
        ),
    )
    .unwrap();
    let challenge = evaluate(&[0, 1, 2, 3, 4], &challenge_ctx);
    assert_eq!(challenge.card_skill_score_up[0], 400.0);
}

#[test]
fn ref_skill_enum() {
    let mut cards = base_cards(1000, 0.0);
    cards[0].skill = lookup_skill(skill(100.0));
    cards[0].skill.resolved[Unit::Ref as usize][0] = ref_skill(80.0, 50.0, 100.0);
    cards[1].skill = lookup_skill(skill(300.0));
    cards[2].skill = lookup_skill(skill(200.0));
    cards[3].skill = lookup_skill(skill(100.0));
    cards[4].skill = lookup_skill(skill(50.0));
    let pool = pool(cards);

    let mut p = params(None, LiveType::Solo, ScoreTarget::Skill);
    p.skill_reference_strategy = SkillReferenceStrategy::Max;
    let max_ctx = DeckContext::new(&pool, p.clone()).unwrap();
    let max_score = evaluate(&[0, 1, 2, 3, 4], &max_ctx);
    output("ref_skill_enum", max_score);
    assert_eq!(max_score.chosen_mask, 1);
    assert!(max_score.card_skill_score_up.contains(&180.0));

    p.skill_reference_strategy = SkillReferenceStrategy::Min;
    let min_score = evaluate(
        &[0, 1, 2, 3, 4],
        &DeckContext::new(&pool, p.clone()).unwrap(),
    );
    assert!(min_score.card_skill_score_up.contains(&105.0));

    p.skill_reference_strategy = SkillReferenceStrategy::Average;
    let avg_score = evaluate(&[0, 1, 2, 3, 4], &DeckContext::new(&pool, p).unwrap());
    assert!(avg_score.card_skill_score_up.contains(&148.75));
}

#[test]
fn custom_mixed() {
    let mut cards = base_cards(1000, 0.0);
    cards[0].event_bonus.base_bonus = 0.0;
    cards[1].event_bonus.base_bonus = 0.0;
    cards[1].attr = Attr::Cool;
    cards[2].character_id = 21;
    cards[2].support_unit = Unit::None;
    cards[2].attr = Attr::Cute;
    let pool = pool(cards);
    let mut support_unit_by_char = [Unit::None; 27];
    support_unit_by_char[21] = Unit::Idol;
    let custom = CustomBonusParams {
        character_mask: (1 << 1) | (1 << 21),
        attr: Some(Attr::Cool),
        support_unit_by_char,
    };
    let event = EventContext {
        event_id: 0,
        custom_bonus: Some(custom),
        ..event(EventType::Marathon)
    };
    let ctx = DeckContext::new(
        &pool,
        params(Some(event), LiveType::Solo, ScoreTarget::Bonus),
    )
    .unwrap();
    let score = evaluate(&[0, 1, 2, 3, 4], &ctx);
    output("custom_mixed", score);
    assert_eq!(score.card_event_bonus_rates[0], 50.0);
    assert_eq!(score.card_event_bonus_rates[1], 25.0);
    assert_eq!(score.card_event_bonus_rates[2], 25.0);
}

#[test]
fn mysekai() {
    let mut cards = base_cards(1000, 0.0);
    for card in &mut cards {
        card.event_bonus.base_bonus = 10.0;
    }
    cards[0].skill = lookup_skill(skill(100.0));
    cards[0].skill.resolved[Unit::Ref as usize][0] = ref_skill(80.0, 50.0, 100.0);
    let pool = pool(cards);
    let event = EventContext {
        event_type: EventType::WorldBloom,
        world_bloom: Some(WorldBloomContext {
            support_deck_count: 2,
            diff_attr_bonus_table: [0.0; ATTR_COUNT],
            support_cards: vec![
                SupportDeckCard {
                    card_id: 10,
                    bonus: 1.0,
                },
                SupportDeckCard {
                    card_id: 11,
                    bonus: 1.0,
                },
            ],
            final_chapter_support: Vec::new(),
            power_total_cap: None,
        }),
        ..event(EventType::WorldBloom)
    };
    let ctx = DeckContext::new(
        &pool,
        params(Some(event), LiveType::Mysekai, ScoreTarget::Mysekai),
    )
    .unwrap();
    let score = evaluate(&[0, 1, 2, 3, 4], &ctx);
    output("mysekai", score);
    assert_eq!(score.mysekai_event_point, 500);
    assert_eq!(score.mysekai_internal_point, 760);
    assert_eq!(score.target_value, 760.0);
    assert_eq!(score.chosen_mask, 0);
    assert_eq!(score.live_score, 0);
    assert_eq!(score.event_point, 0);
}

#[test]
fn no_event() {
    let pool = pool(base_cards(1000, 0.0));
    let ctx = DeckContext::new(&pool, params(None, LiveType::Solo, ScoreTarget::Score)).unwrap();
    let score = evaluate(&[0, 1, 2, 3, 4], &ctx);
    output("no_event", score);
    assert_eq!(score.live_score, 20000);
    assert_eq!(score.event_point, score.live_score);
    assert_eq!(score.diff_attr_bonus_rate, 0.0);
}

#[test]
fn challenge_auto() {
    let pool = pool(base_cards(1000, 0.0));
    let challenge = DeckContext::new(
        &pool,
        params(
            Some(event(EventType::Marathon)),
            LiveType::Challenge,
            ScoreTarget::Score,
        ),
    )
    .unwrap();
    let challenge_auto = DeckContext::new(
        &pool,
        params(
            Some(event(EventType::Marathon)),
            LiveType::ChallengeAuto,
            ScoreTarget::Score,
        ),
    )
    .unwrap();
    let score = evaluate(&[0, 1, 2, 3, 4], &challenge);
    output("challenge_auto", score);
    let auto_score = evaluate(&[0, 1, 2, 3, 4], &challenge_auto);
    assert_eq!(score.event_point, auto_score.event_point);
    assert_eq!(score.event_point, 12120);
}

#[test]
fn cheerful() {
    let pool = pool(base_cards(1000, 0.0));
    let ctx = DeckContext::new(
        &pool,
        params(
            Some(event(EventType::CheerfulCarnival)),
            LiveType::Multi,
            ScoreTarget::Score,
        ),
    )
    .unwrap();
    let score = evaluate(&[0, 1, 2, 3, 4], &ctx);
    output("cheerful", score);
    assert_eq!(score.live_score, 20000);
    assert_eq!(score.event_point, 149);
}

#[test]
fn best_skill_order() {
    let pool = skill_order_pool();
    let average = evaluate(
        &[0, 1, 2, 3, 4],
        &DeckContext::new(&pool, skill_order_params(LiveSkillOrder::Average)).unwrap(),
    );
    let best = evaluate(
        &[0, 1, 2, 3, 4],
        &DeckContext::new(&pool, skill_order_params(LiveSkillOrder::Best)).unwrap(),
    );
    output("best_skill_order", best);
    assert_eq!(average.live_score, 110000);
    assert_eq!(best.live_score, 130000);
    assert!(best.live_score > average.live_score);
}

#[test]
fn worst_skill_order() {
    let pool = skill_order_pool();
    let average = evaluate(
        &[0, 1, 2, 3, 4],
        &DeckContext::new(&pool, skill_order_params(LiveSkillOrder::Average)).unwrap(),
    );
    let best = evaluate(
        &[0, 1, 2, 3, 4],
        &DeckContext::new(&pool, skill_order_params(LiveSkillOrder::Best)).unwrap(),
    );
    let worst = evaluate(
        &[0, 1, 2, 3, 4],
        &DeckContext::new(&pool, skill_order_params(LiveSkillOrder::Worst)).unwrap(),
    );
    output("worst_skill_order", worst);
    assert_eq!(worst.live_score, 90000);
    assert!(worst.live_score < average.live_score);
    assert!(worst.live_score < best.live_score);
}
