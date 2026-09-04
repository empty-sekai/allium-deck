# Parameter reference

Build parameters are the fourth argument of `engine::recommend_json` (a JSON object) or, for the typed entry point `engine::recommend`, the `handler::BuildParams` struct. The JSON parser accepts both camelCase and snake_case for every key listed with two spellings; unknown keys are ignored. Values outside the documented ranges are rejected with a parse error.

## General

| Key | Type | Default | Notes |
| --- | --- | --- | --- |
| `region` | string | `"cn"` | Region tag carried through to the caller; does not change engine math. |
| `target` | string | `"score"` | `score`, `power`, `skill`, `bonus`, `mysekai`. Event point optimization is `score` plus an event context. |
| `liveType` / `live_type` | string | `"solo"` | `solo`, `auto`, `multi`, `cheerful`, `challenge`, `challenge_auto`, `mysekai`. |
| `limit` | int | 10 | Number of decks returned (Top-K). Distinct card sets. |
| `member` | int | absent | Compatibility field; only 5 (or absent) is supported. |
| `timeoutMs` / `timeout_ms` | int | 300000 | Search deadline in milliseconds, max 300000. On expiry the best results found so far are returned (anytime behavior); exactness is only guaranteed when the search finishes before the deadline. |
| `minimize` | bool | false | Weakest-deck search. Only meaningful with `target=power`; ignored otherwise. |

## Event context

| Key | Type | Default | Notes |
| --- | --- | --- | --- |
| `eventId` / `event_id` | int | absent | Real event ID from masterdata. Enables event-point scoring for `target=score`. |
| `eventType` / `event_type` | string | absent | Simulated event type when no `eventId` is given: `marathon`, `cheerful`/`cheerful_carnival`, `world_bloom`/`wl`. |
| `eventUnit` / `event_unit` | string | absent | Simulated event unit: `light_sound`, `idol`, `street`, `theme_park`, `school_refusal`, `piapro`. |
| `eventAttr` / `event_attr` | string | absent | Simulated event attribute: `mysterious`, `cute`, `cool`, `pure`, `happy`. |
| `customBonusCharacterIds` / `custom_bonus_character_ids` | int[] | `[]` | Mixed-event character set; overrides the unit expansion of `eventUnit`. IDs 1–26, max 26 entries. |
| `customBonusAttr` / `custom_bonus_attr` | string | absent | Mixed-event attribute. |
| `customBonusCharacterSupportUnits` / `custom_bonus_character_support_units` | array | `[]` | Support-unit constraints for Virtual Singer entries in the custom character set. |
| `boost` | int | absent | Energy flame count (0–10), not a multiplier; affects event point display math. |
| `targetBonusList` / `target_bonus_list` | int[] | `[]` | For `target=bonus`: exact event-bonus tiers to hit, one Top-K per tier. Tiered bonus search builds its own candidate pool from the whole box — cards above the highest requested tier are dropped, hard constraints (fixed/excluded cards, unit/attribute filters) still apply, and every remaining card stays searchable so low-granularity tiers stay reachable. |

## World Bloom

| Key | Type | Default | Notes |
| --- | --- | --- | --- |
| `worldBloomCharacterId` / `world_bloom_character_id` | int | absent | Chapter character. |
| `worldBloomEventTurn` / `world_bloom_event_turn` | int | absent | Chapter turn. |
| `worldBloomFinaleTurn` / `world_bloom_finale_turn` | int 2\|3 | absent | 模拟 WL 终章：2 走 legacy 终章 180，3 合成模拟终章 3_200_000。需配合 `worldBloomCharacterId`。 |
| `forcedLeaderCharacterId` | int | absent | Final chapter only: fixes the leader character; ignored elsewhere. |
| `supportMasterMax` / `support_master_max` | bool | false | Value support-deck cards at max master rank. |
| `supportSkillMax` / `support_skill_max` | bool | false | Value support-deck cards at max skill level. |

## Deck constraints

| Key | Type | Default | Notes |
| --- | --- | --- | --- |
| `fixedCards` | int[] | `[]` | Card IDs locked into the deck. Combined with `fixedCharacters`, at most 5 slots; slot 0 carries leader semantics. |
| `fixedCharacters` | int[] | `[]` | Character IDs locked into slots after the fixed cards. |
| `excludedCards` | int[] | `[]` | Card IDs removed from the candidate pool. |
| `challengeLiveCharacterId` / `challenge_live_character_id` | int | absent | Character for `challenge` / `challenge_auto` (character uniqueness is disabled there). |
| `unitFilter` / `unit_filter` | string | absent | Hard unit filter on the pool. VS cards match by support unit. |
| `attrFilter` / `attr_filter` | string | absent | Hard attribute filter on the pool. |
| `filterOtherUnit` | bool | false | Keep only event-unit members; requires an event unit context. |

## Music

| Key | Type | Default | Notes |
| --- | --- | --- | --- |
| `musicId` / `music_id` | int | absent | Song for score math. Without it, the no-music fallback table is used. |
| `musicDiff` / `music_diff` | string | absent | Difficulty; `expert` when omitted. |

## Skill handling

| Key | Type | Default | Notes |
| --- | --- | --- | --- |
| `bestSkillAsLeader` | bool | true | Prefer the strongest skill in the leader slot. |
| `liveSkillOrder` / `skillOrderChooseStrategy` / `skill_order_choose_strategy` | string | `"average"` | `best`/`max`, `worst`/`min`, `average`, `specific`. Default reflects the in-game expectation (skill order is not player-controlled); `best` gives an optimistic upper bound. |
| `specificSkillOrder` | int[5] | absent | Required when order is `specific`: five distinct slot indices. |
| `skillReferenceChooseStrategy` / `skillReferenceStrategy` | string | `"average"` | Reference-skill valuation: `max`, `min`, `average`. |
| `keepAfterTrainingState` | bool | false | Lock each card's current trained/untrained art state; cultivation overrides do not flip it. |

## Multi / Cheerful context

| Key | Type | Default | Notes |
| --- | --- | --- | --- |
| `multiLiveTeammatePower` / `multi_teammate_power` | int | absent | Teammate total power for multi/cheerful score math. |
| `multiLiveTeammateScoreUp` / `multi_teammate_score_up` | int | absent | Teammate effective skill value. |
| `multiLiveScoreUpLowerBound` / `multi_live_score_up_lower_bound` | float | absent | Lower bound on total effective skill across the deck. |
| `otherScore` / `other_score` | int | absent | Opponent score (cheerful). |
| `life` | int | absent | Life value (cheerful). |

## Card cultivation configs

Per-rarity defaults (`rarity1Config` … `rarity4Config`, `rarityBirthdayConfig`, snake_case accepted) and per-card overrides (`singleCardConfigs`, each `{cardId, config}` or flat). Priority: single-card override > rarity default. Each config object:

| Key | Type | Default | Notes |
| --- | --- | --- | --- |
| `disable` | bool | false | Exclude this class of cards. |
| `levelMax` | bool | false | Value at max level (trainable cards are valued in trained state). |
| `level` | int | absent | Exact level; overrides `levelMax`. |
| `skillMax` | bool | false | Value at max skill level. |
| `skillLevel` | int | absent | Exact skill level; overrides `skillMax`. |
| `episodeRead` | bool | false | Value with story episodes read. |
| `episodeReadCount` | int | absent | Exact episodes read; overrides `episodeRead`. |
| `masterMax` | bool | false | Value at max master rank. |
| `masterRank` | int | absent | Exact master rank; overrides `masterMax`. |
| `canvas` | bool | false | Apply MySekai canvas bonus. |

## Exactness by mode

"Exact" means the returned Top-K score sequence provably matches full enumeration (verified against a brute-force reference in the test suite). Tied decks are interchangeable: when several sets share a score, the engine's representative may differ from another enumeration order.

| Mode | Path | Guarantee |
| --- | --- | --- |
| `score` (with or without event, incl. World Bloom chapters, `mysekai`) | dominance pruning + branch-and-bound DFS, Top-K alternatives expansion | Exact, including Top-K |
| `score` World Bloom final chapter, fixed leader character or fixed leader card | grouped character search / DFS + member alternatives, leader rotations | Exact, including Top-K |
| `score` World Bloom final chapter, auto leader | leader-key truncation (3 per character) + beam seeding + grouped search | Heuristic: strong in practice, no exactness proof |
| `challenge` / `challenge_auto` | full enumeration with admissible bound pruning (no card elimination) | Exact, including Top-K |
| `power` (no fixed cards/characters, not `minimize`) | 49-scenario additive DP | Exact, including Top-K |
| `power` (with fixed cards/characters, or `minimize`) | quality-prefix truncation (28 cards, ≤6 per character) | Heuristic |
| `skill` | quality-prefix truncation (20 cards, ≤3 per character) | Heuristic |
| `bonus` (`targetBonusList`) | dedicated candidate pool + DFS per bonus tier | Exact per tier |

Dominance pruning compares power, skill, event bonus, attribute, unit mask — and, in World Bloom, the per-leader support-deck penalty (a support-listed card placed in the deck forfeits its support bonus). Timeouts turn any exact path into best-effort.
