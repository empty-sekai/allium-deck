#![cfg(any())]

mod testdata_adapter;

#[path = "scenario_test/framework.rs"]
mod framework;
#[path = "scenario_test/scenarios.rs"]
mod scenarios;
#[path = "scenario_test/stats.rs"]
mod stats;
#[path = "scenario_test/verify.rs"]
mod verify;

use framework::prepare_scenario_cases;
use scenarios::{get_scenario, SCENARIOS};
use stats::print_summary;
use verify::{run_case, CaseStatus};

fn run_scenario(name: &str) {
    let scenario = get_scenario(name).unwrap_or_else(|| panic!("未知 scenario: {name}"));
    let cases = prepare_scenario_cases(scenario)
        .unwrap_or_else(|err| panic!("准备 scenario {name} 失败: {err}"));
    let runs = cases.iter().map(run_case).collect::<Vec<_>>();
    print_summary(name, &runs);

    let bugs = runs
        .iter()
        .filter(|run| matches!(run.status, CaseStatus::Bug))
        .map(|run| {
            format!(
                "{}: {} (elapsed={:.3}ms, pool={})",
                run.case_name, run.detail, run.elapsed_ms, run.pool_size
            )
        })
        .collect::<Vec<_>>();
    assert!(bugs.is_empty(), "scenario {name} 失败:\n{}", bugs.join("\n"));
}

#[test]
fn scenario_score_multi_ev() {
    run_scenario("score_multi_ev");
}

#[test]
fn scenario_score_multi_noev() {
    run_scenario("score_multi_noev");
}

#[test]
fn scenario_score_multi_fast() {
    run_scenario("score_multi_fast");
}

#[test]
fn scenario_score_noev_fast() {
    run_scenario("score_noev_fast");
}

#[test]
fn scenario_power_solo_ev() {
    run_scenario("power_solo_ev");
}

#[test]
fn scenario_power_solo_fast() {
    run_scenario("power_solo_fast");
}

#[test]
fn scenario_skill_auto_ev() {
    run_scenario("skill_auto_ev");
}

#[test]
fn scenario_bonus_multi_ev() {
    run_scenario("bonus_multi_ev");
}

#[test]
fn scenario_score_solo_ev() {
    run_scenario("score_solo_ev");
}

#[test]
fn scenario_score_auto_ev() {
    run_scenario("score_auto_ev");
}

#[test]
fn scenario_score_cheerful() {
    run_scenario("score_cheerful");
}

#[test]
fn scenario_power_noev() {
    run_scenario("power_noev");
}

#[test]
fn scenario_bonus_wl() {
    run_scenario("bonus_wl");
}

#[test]
fn scenario_mysekai() {
    run_scenario("mysekai");
}

#[test]
fn scenario_score_final_chapter() {
    run_scenario("score_final_chapter");
}

#[test]
fn scenario_bonus_final_chapter() {
    run_scenario("bonus_final_chapter");
}

#[test]
fn scenario_score_fixed_card() {
    run_scenario("score_fixed_card");
}

#[test]
fn scenario_score_fixed_char() {
    run_scenario("score_fixed_char");
}

#[test]
fn scenario_score_multi_ev_diff_music() {
    run_scenario("score_multi_ev_diff_music");
}

#[test]
fn scenario_summary() {
    for scenario in SCENARIOS {
        run_scenario(scenario.name);
    }
}
