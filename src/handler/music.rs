use super::types::{BuildParams, GameData, MusicMeta};

/// handler 构建出的歌曲参数。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MusicParams {
    /// 活动倍率（扩大 100 倍）。
    pub event_rate_pct: u32,
    /// 歌曲主表。
    pub meta: MusicMeta,
}

/// 构建歌曲参数。
pub(crate) fn build_music_params(game: &GameData<'_>, params: &BuildParams) -> Option<MusicParams> {
    let music_id = params.music_id?;
    let meta = game
        .music_metas
        .iter()
        .find(|entry| entry.music_id == music_id)?
        .clone();
    let diff_rate = params.music_diff.as_deref().and_then(|diff| {
        game.music_difficulties
            .iter()
            .find(|entry| entry.music_id == music_id && entry.difficulty.eq_ignore_ascii_case(diff))
            .and_then(|entry| entry.event_rate)
    });
    let event_rate_pct = diff_rate.unwrap_or(match params.live_type {
        crate::types::LiveType::Multi | crate::types::LiveType::Cheerful => meta.event_rate_multi,
        crate::types::LiveType::Auto | crate::types::LiveType::ChallengeAuto => {
            meta.event_rate_auto
        }
        _ => meta.event_rate_solo,
    });

    Some(MusicParams {
        event_rate_pct: event_rate_pct.max(0) as u32,
        meta,
    })
}
