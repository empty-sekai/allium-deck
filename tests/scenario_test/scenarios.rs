/// 场景构造方式。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScenarioKind {
    LegacyCombo,
    ScoreSoloEv,
    ScoreAutoEv,
    ScoreCheerful,
    PowerNoev,
    BonusWl,
    Mysekai,
    ScoreFinalChapter,
    BonusFinalChapter,
    ScoreFixedCard,
    ScoreFixedChar,
    ScoreMultiEvDiffMusic,
}

/// 单个 scenario 定义。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScenarioDef {
    pub name: &'static str,
    pub source_combo: &'static str,
    pub kind: ScenarioKind,
    pub min_cases: usize,
    pub max_cases: Option<usize>,
}

pub const SCENARIOS: &[ScenarioDef] = &[
    ScenarioDef {
        name: "score_multi_ev",
        source_combo: "score_multi_ev",
        kind: ScenarioKind::LegacyCombo,
        min_cases: 5,
        max_cases: None,
    },
    ScenarioDef {
        name: "score_multi_noev",
        source_combo: "score_multi_noev",
        kind: ScenarioKind::LegacyCombo,
        min_cases: 5,
        max_cases: None,
    },
    ScenarioDef {
        name: "score_multi_fast",
        source_combo: "score_multi_fast",
        kind: ScenarioKind::LegacyCombo,
        min_cases: 5,
        max_cases: None,
    },
    ScenarioDef {
        name: "score_noev_fast",
        source_combo: "score_noev_fast",
        kind: ScenarioKind::LegacyCombo,
        min_cases: 5,
        max_cases: None,
    },
    ScenarioDef {
        name: "power_solo_ev",
        source_combo: "power_solo_ev",
        kind: ScenarioKind::LegacyCombo,
        min_cases: 5,
        max_cases: None,
    },
    ScenarioDef {
        name: "power_solo_fast",
        source_combo: "power_solo_fast",
        kind: ScenarioKind::LegacyCombo,
        min_cases: 5,
        max_cases: None,
    },
    ScenarioDef {
        name: "skill_auto_ev",
        source_combo: "skill_auto_ev",
        kind: ScenarioKind::LegacyCombo,
        min_cases: 5,
        max_cases: None,
    },
    ScenarioDef {
        name: "bonus_multi_ev",
        source_combo: "bonus_multi_ev",
        kind: ScenarioKind::LegacyCombo,
        min_cases: 5,
        max_cases: None,
    },
    ScenarioDef {
        name: "score_solo_ev",
        source_combo: "score_multi_ev",
        kind: ScenarioKind::ScoreSoloEv,
        min_cases: 5,
        max_cases: Some(8),
    },
    ScenarioDef {
        name: "score_auto_ev",
        source_combo: "score_multi_ev",
        kind: ScenarioKind::ScoreAutoEv,
        min_cases: 5,
        max_cases: Some(8),
    },
    ScenarioDef {
        name: "score_cheerful",
        source_combo: "score_multi_ev",
        kind: ScenarioKind::ScoreCheerful,
        min_cases: 5,
        max_cases: Some(8),
    },
    ScenarioDef {
        name: "power_noev",
        source_combo: "power_solo_ev",
        kind: ScenarioKind::PowerNoev,
        min_cases: 5,
        max_cases: Some(8),
    },
    ScenarioDef {
        name: "bonus_wl",
        source_combo: "bonus_multi_ev",
        kind: ScenarioKind::BonusWl,
        min_cases: 5,
        max_cases: Some(8),
    },
    ScenarioDef {
        name: "mysekai",
        source_combo: "score_multi_ev",
        kind: ScenarioKind::Mysekai,
        min_cases: 5,
        max_cases: Some(8),
    },
    ScenarioDef {
        name: "score_final_chapter",
        source_combo: "score_multi_ev",
        kind: ScenarioKind::ScoreFinalChapter,
        min_cases: 5,
        max_cases: Some(8),
    },
    ScenarioDef {
        name: "bonus_final_chapter",
        source_combo: "bonus_multi_ev",
        kind: ScenarioKind::BonusFinalChapter,
        min_cases: 5,
        max_cases: Some(8),
    },
    ScenarioDef {
        name: "score_fixed_card",
        source_combo: "score_multi_ev",
        kind: ScenarioKind::ScoreFixedCard,
        min_cases: 5,
        max_cases: Some(8),
    },
    ScenarioDef {
        name: "score_fixed_char",
        source_combo: "score_multi_ev",
        kind: ScenarioKind::ScoreFixedChar,
        min_cases: 5,
        max_cases: Some(8),
    },
    ScenarioDef {
        name: "score_multi_ev_diff_music",
        source_combo: "score_multi_ev",
        kind: ScenarioKind::ScoreMultiEvDiffMusic,
        min_cases: 5,
        max_cases: Some(10),
    },
];

pub fn get_scenario(name: &str) -> Option<&'static ScenarioDef> {
    SCENARIOS.iter().find(|scenario| scenario.name == name)
}
