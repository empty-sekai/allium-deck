//! Criterion 基准：冷建池 + 活动搜索 + 无活动搜索。
//!
//! 数据来自 `synth_masterdata`（确定性合成，规模贴近真实：26 角色 / 1300 卡，
//! 满配账号建池后约 260 张），不依赖任何游戏 masterdata，`cargo bench`
//! 对任何人开箱可跑。合成数据的绝对耗时与真实数据不可直接比较，这套基准
//! 用于跨提交的相对回归对比。
//!
//! 无活动多人搜索是分支定界的最坏情形（无活动加成可剪枝），全量池单次约
//! 数十秒，默认基准用 `attrFilter` 收窄到单属性池（精确完成，亚秒级）；
//! 全量版本设 `ALLIUM_BENCH_FULL_NO_EVENT=1` 后运行（约数分钟）。
//! 注意 `SearchParams::timeout_ms` 目前无法截断长搜索（见 dfs.rs `timed_out`：
//! 超时只剪当前子树、无粘性中止标志），因此不能靠 timeout 控制基准时长。

mod synth_masterdata;

use allium_deck::engine::{
    parse_build_params_json, parse_user_profile_json, MasterdataSources, OwnedGameData,
};
use allium_deck::handler::{build_card_pool, BuildParams, UserProfile};
use allium_deck::search::{search, SearchParams};
use criterion::{criterion_group, criterion_main, Criterion, SamplingMode};
use std::time::Duration;

const EVENT_PARAMS: &str =
    r#"{"eventId":1,"eventType":"marathon","liveType":"multi","target":"score","limit":8}"#;
const NO_EVENT_PARAMS: &str = r#"{"liveType":"multi","target":"score","limit":8}"#;
/// 无活动 + 单属性池：把最坏情形收窄到可精确完成的规模。
const NO_EVENT_ATTR_PARAMS: &str =
    r#"{"liveType":"multi","target":"score","limit":8,"attrFilter":"cool"}"#;

struct BenchInput {
    owned: OwnedGameData,
    user: UserProfile,
}

fn build_input() -> BenchInput {
    let synth = synth_masterdata::generate(synth_masterdata::DEFAULT_SEED);
    let sources = MasterdataSources::from_strings(synth.tables, synth.music_metas_json);
    let owned = OwnedGameData::from_sources(&sources).expect("synth masterdata parses");
    let user = parse_user_profile_json(&synth.user_json).expect("synth user parses");
    BenchInput { owned, user }
}

fn params(json: &str) -> BuildParams {
    parse_build_params_json(json).expect("bench params parse")
}

const SEARCH_PARAMS: SearchParams = SearchParams {
    top_k: 8,
    timeout_ms: 300_000,
};

/// 冷建池：每次迭代从 `GameData` 全量建池（索引、养成态展开、支配剪枝、支援卡组）。
fn bench_cold_build(c: &mut Criterion) {
    let input = build_input();
    let game = input.owned.as_ref();
    let event_params = params(EVENT_PARAMS);
    let no_event_params = params(NO_EVENT_PARAMS);

    let mut group = c.benchmark_group("build_pool");
    group.bench_function("event_multi", |b| {
        b.iter(|| build_card_pool(&input.user, &game, &event_params).expect("build pool"))
    });
    group.bench_function("no_event_multi", |b| {
        b.iter(|| build_card_pool(&input.user, &game, &no_event_params).expect("build pool"))
    });
    group.finish();
}

/// 活动多人搜索：活动加成让上界剪枝高度有效，真实与合成数据都在亚毫秒级。
fn bench_event_search(c: &mut Criterion) {
    let input = build_input();
    let game = input.owned.as_ref();
    let (pool, ctx) =
        build_card_pool(&input.user, &game, &params(EVENT_PARAMS)).expect("build pool");

    c.bench_function("search/event_multi", |b| {
        b.iter(|| search(&pool, &ctx, &SEARCH_PARAMS))
    });
}

/// 无活动多人搜索（单属性池）：无加成可剪枝的最坏情形，收窄后可精确完成。
fn bench_no_event_search(c: &mut Criterion) {
    let input = build_input();
    let game = input.owned.as_ref();
    let (pool, ctx) =
        build_card_pool(&input.user, &game, &params(NO_EVENT_ATTR_PARAMS)).expect("build pool");

    let mut group = c.benchmark_group("search");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(15));
    group.sampling_mode(SamplingMode::Flat);
    group.bench_function("no_event_multi_attr_cool", |b| {
        b.iter(|| search(&pool, &ctx, &SEARCH_PARAMS))
    });
    group.finish();
}

/// 全量无活动多人搜索：单次迭代约数十秒，默认跳过；
/// `ALLIUM_BENCH_FULL_NO_EVENT=1 cargo bench -- no_event_multi_full` 启用。
fn bench_no_event_search_full(c: &mut Criterion) {
    if std::env::var_os("ALLIUM_BENCH_FULL_NO_EVENT").is_none() {
        return;
    }
    let input = build_input();
    let game = input.owned.as_ref();
    let (pool, ctx) =
        build_card_pool(&input.user, &game, &params(NO_EVENT_PARAMS)).expect("build pool");

    let mut group = c.benchmark_group("search");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(300));
    group.sampling_mode(SamplingMode::Flat);
    group.bench_function("no_event_multi_full", |b| {
        b.iter(|| search(&pool, &ctx, &SEARCH_PARAMS))
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_cold_build,
    bench_event_search,
    bench_no_event_search,
    bench_no_event_search_full
);
criterion_main!(benches);
