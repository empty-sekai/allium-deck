use super::types::{BuildParams, GameData, MusicMeta};

/// handler 构建出的歌曲参数。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MusicParams {
    /// 活动倍率（扩大 100 倍）。
    pub event_rate_pct: u32,
    /// 歌曲主表。
    pub meta: MusicMeta,
}

/// 不指定难度时的默认难度。base_score/skill_scores 分难度（影响排序与显示分数），
/// music_metas.json 每首歌按难度有多行，旧逻辑 `find(music_id)` 取首行（约 96% 的歌首行是
/// easy），导致默认按 easy 偏低算分。玩家组卡口径是 expert，故默认取 expert 行。
const DEFAULT_MUSIC_DIFFICULTY: &str = "expert";

/// 构建歌曲参数。
pub(crate) fn build_music_params(game: &GameData<'_>, params: &BuildParams) -> Option<MusicParams> {
    let music_id = params.music_id?;
    let target_diff = params
        .music_diff
        .as_deref()
        .unwrap_or(DEFAULT_MUSIC_DIFFICULTY);
    // 按目标难度选 meta 行；选不到（该歌缺此难度）再回落到该歌任意行，保证有结果。
    // 每行的 event_rate_* 已是该难度的值（engine 构建时按行填入），故选对行即得对的倍率，
    // 不再需要单独查 music_difficulties 表。
    let meta = game
        .music_metas
        .iter()
        .find(|entry| {
            entry.music_id == music_id && entry.difficulty.eq_ignore_ascii_case(target_diff)
        })
        .or_else(|| {
            game.music_metas
                .iter()
                .find(|entry| entry.music_id == music_id)
        })?
        .clone();
    let event_rate_pct = match params.live_type {
        crate::types::LiveType::Multi | crate::types::LiveType::Cheerful => meta.event_rate_multi,
        crate::types::LiveType::Auto | crate::types::LiveType::ChallengeAuto => {
            meta.event_rate_auto
        }
        _ => meta.event_rate_solo,
    };

    Some(MusicParams {
        event_rate_pct: event_rate_pct.max(0) as u32,
        meta,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::OwnedGameData;

    /// 构造一行指定难度的 meta；base_score 用来区分选中了哪一行。
    fn meta(music_id: i32, difficulty: &str, base_score: f64, event_rate: i32) -> MusicMeta {
        MusicMeta {
            music_id,
            difficulty: difficulty.to_string(),
            event_rate_solo: event_rate,
            event_rate_multi: event_rate,
            event_rate_auto: event_rate,
            base_score,
            base_score_auto: base_score,
            fever_score: 0.0,
            solo_skill_scores: [0.0; 6],
            multi_skill_scores: [0.0; 6],
            auto_skill_scores: [0.0; 6],
            music_time: 100.0,
            tap_count: 500,
        }
    }

    /// music_metas.json 的真实行序：easy 在前、expert 在后。验证默认选 expert 而非首行。
    fn game_with_diffs() -> OwnedGameData {
        OwnedGameData {
            music_metas: vec![
                meta(74, "easy", 1.0050, 100),
                meta(74, "normal", 1.0344, 100),
                meta(74, "hard", 1.0696, 100),
                meta(74, "expert", 1.1099, 100),
                meta(74, "master", 1.1331, 100),
            ],
            ..OwnedGameData::default()
        }
    }

    #[test]
    fn build_music_params_defaults_to_expert_not_first_row() {
        // 不指定难度时必须选 expert 行（base_score=1.1099），而非文件首行 easy（1.0050）。
        let owned = game_with_diffs();
        let params = BuildParams {
            music_id: Some(74),
            music_diff: None,
            ..BuildParams::default()
        };
        let result = build_music_params(&owned.as_ref(), &params).expect("应有结果");
        assert_eq!(result.meta.difficulty, "expert");
        assert!((result.meta.base_score - 1.1099).abs() < 1e-9);
    }

    #[test]
    fn build_music_params_selects_explicit_difficulty() {
        let owned = game_with_diffs();
        let params = BuildParams {
            music_id: Some(74),
            music_diff: Some("master".to_string()),
            ..BuildParams::default()
        };
        let result = build_music_params(&owned.as_ref(), &params).expect("应有结果");
        assert_eq!(result.meta.difficulty, "master");
        assert!((result.meta.base_score - 1.1331).abs() < 1e-9);
    }

    #[test]
    fn build_music_params_falls_back_when_difficulty_missing() {
        // 该歌只有 easy 行，请求 expert（默认）应回落到现有行而非返回 None。
        let owned = OwnedGameData {
            music_metas: vec![meta(99, "easy", 1.0, 100)],
            ..OwnedGameData::default()
        };
        let params = BuildParams {
            music_id: Some(99),
            music_diff: None,
            ..BuildParams::default()
        };
        let result = build_music_params(&owned.as_ref(), &params).expect("应回落到 easy 行");
        assert_eq!(result.meta.difficulty, "easy");
    }
}
