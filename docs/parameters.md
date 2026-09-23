# Parameter reference

Build parameters are the fourth argument of `engine::recommend_json` (a JSON object) or, for the typed entry point `engine::recommend`, the `handler::BuildParams` struct. The JSON parser accepts both camelCase and snake_case for every key listed with two spellings; unknown keys are ignored. Values outside the documented ranges are rejected with a parse error.

## General

| Key | Type | Default | Notes |
| --- | --- | --- | --- |
| `region` | string | `"cn"` | Region tag carried through to the caller; does not change engine math. |
| `target` | string | `"score"` | `score`, `power`, `skill`, `bonus`, `mysekai`. Event point optimization is `score` plus an event context. `score` is normalized to `mysekai` when `liveType` is `mysekai`, because a MySekai live has no live score; `bonus` and `power` results under that live type report live score 0 and break ties by the canonical order below. |
| `liveType` / `live_type` | string | `"solo"` | `solo`, `auto`, `multi`, `cheerful`, `challenge`, `challenge_auto`, `mysekai`. |
| `limit` | int | 10 | Number of decks returned (Top-K). Distinct card sets. Max 100. |
| `member` | int | absent | Compatibility field; only 5 (or absent) is supported. |
| `timeoutMs` / `timeout_ms` | int | 300000 | Public JSON search deadline in milliseconds; valid range `1..=300000`. On expiry the legal incumbents found so far are returned (anytime behavior); exactness is certified only when completion is `Complete`. The lower-level `SearchParams` API additionally reserves `0` for an unlimited internal search. |
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
| `customBonusCharacterSupportUnits` / `custom_bonus_character_support_units` | object | `{}` | Support-unit constraints for Virtual Singer entries in the custom character set, keyed by character id (`{"21": "street"}`). |
| `boost` | int | absent | Energy flame count (0–10), not a multiplier; affects event point display math. Values outside 0–10 are rejected. |
| `targetBonusList` / `target_bonus_list` | int[] | `[]` | For `target=bonus`: exact event-bonus tiers to hit, one Top-K per tier. Max 32 tiers, each 0–10000. An empty result for a tier means the tier is unreachable with the given box. Tiered bonus search builds its own candidate pool from the whole box — only cards whose unavoidable base contribution (or full card bonus when the event counts every limited bonus) exceeds the highest requested tier are dropped, hard constraints (fixed/excluded cards, unit/attribute filters) still apply, and every remaining card stays searchable so low-granularity tiers stay reachable. |

## World Bloom

| Key | Type | Default | Notes |
| --- | --- | --- | --- |
| `worldBloomCharacterId` / `world_bloom_character_id` | int | absent | Chapter character. |
| `worldBloomEventTurn` / `world_bloom_event_turn` | int | absent | Chapter turn (1, 2 or 3). Turn 1/2 require `eventUnit`; turn 3 requires `worldBloomCharacterId`. |
| `worldBloomFinaleTurn` / `world_bloom_finale_turn` | int 2\|3 | absent | 模拟 WL 终章：2 走 legacy 终章 180，3 合成模拟终章 3_200_000。需配合 `worldBloomCharacterId`。 |
| `forcedLeaderCharacterId` | int | absent | Final chapter only: fixes the leader character; ignored elsewhere. |
| `supportMasterMax` / `support_master_max` | bool | false | Value support-deck cards at max master rank. |
| `supportSkillMax` / `support_skill_max` | bool | false | Value support-deck cards at max skill level. |

Final chapter leader honor: a deck equips one main honor, so the leader-only honor bonus is a single event-honor row matched by event, honor and leader character; owned honors do not stack. For each leader character the builder assumes the owned honor with the largest leader bonus, breaking ties by the smallest honor ID. Results report it as `main_honor_id` (`mainHonorId` in the HTTP service); the key is omitted outside the final chapter or when the leader character has no matching owned honor. The honor power bonus added to total power is computed separately and still covers every owned honor.

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
| `musicId` / `music_id` | int | absent | Song for score math. Without it, the no-music fallback table is used. `10000` (`engine::OMAKASE_MUSIC_ID`) selects the omakase song, whose metas average every master, expert and hard row. |
| `musicDiff` / `music_diff` | string | absent | Difficulty; `expert` when omitted. |

## Skill handling

| Key | Type | Default | Notes |
| --- | --- | --- | --- |
| `bestSkillAsLeader` | bool | true | Prefer the strongest skill in the leader slot. |
| `liveSkillOrder` / `skillOrderChooseStrategy` / `skill_order_choose_strategy` | string | `"average"` | `best`/`max`, `worst`/`min`, `average`, `specific`. Default reflects the in-game expectation (skill order is not player-controlled); `best` gives an optimistic upper bound. |
| `specificSkillOrder` | int[5] | absent | Required when order is `specific`: five distinct slot indices. |
| `skillReferenceChooseStrategy` / `skillReferenceStrategy` | string | `"average"` | Reference-skill valuation: `max`, `min`, `average`. |
| `keepAfterTrainingState` | bool | false | Lock each card's current trained/untrained art state; cultivation overrides do not flip it. |

Skills that depend on the deck are resolved for each evaluated deck, the same way whether or not `keepAfterTrainingState` is set. The art state only selects which skill of a card with an after-training skill applies: the original art uses the base skill, the trained art the after-training skill.

- Different-unit skills count the distinct units of the other four members whose unit differs from the card's own unit, up to two. A Virtual Singer card with a support unit counts as that unit; one without a support unit counts as `piapro`.
- Reference skills add `min(target × rate / 100, max)` for one other member, chosen by `skillReferenceChooseStrategy` (`average` takes the mean over the other four members). The share is not rounded. `target` is the static maximum of that member's skill at its skill level: every conditional part is taken at its largest row (the highest character-rank row, the full same-unit enhancement, two counted units, the largest reference addition), whatever the deck or the owner's character rank.

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

"Exact" means the returned distinct Top-K results match full legal enumeration under the canonical ordering below. Equal objective values do not make public card sets interchangeable. With the same immutable pool, context and a completed search, increasing `limit` preserves the smaller result list as a prefix.

| Mode | Path | Guarantee |
| --- | --- | --- |
| `score` (with or without event, incl. World Bloom chapters, `mysekai`) | dominance pruning + admissible B&B, Top-K alternatives expansion; WL uses an attribute/character matching relaxation | Exact, including Top-K |
| `score` World Bloom final chapter, fixed leader character or fixed leader card | grouped character/card B&B + exact attribute-union DP bound + member alternatives / leader rotations | Exact, including Top-K |
| `score` World Bloom final chapter, auto leader | every leader variant becomes a proof-carrying job, searched in ceiling order; grouped admissible B&B + alternatives / rotations | Exact, including Top-K |
| `challenge` / `challenge_auto` | full feasible-set search with admissible bound pruning, merged per character for challenge-all | Exact, including Top-K |
| `power` (no fixed cards/characters, not `minimize`) | 49-scenario additive DP | Exact, including Top-K |
| `power` (with fixed cards/characters, or `minimize`) | full-candidate B&B with an admissible power upper bound / minimization lower bound | Exact, including Top-K |
| `skill` | full-candidate B&B with a per-card skill-max relaxation | Exact, including Top-K |
| `bonus` (`targetBonusList`) | dedicated exact-reachability DFS per bonus tier | Exact per tier, including Top-K |

Leader-only honor and limited bonuses also remain in integer tenths throughout preparation and evaluation. Only upper bounds round them upward to whole percentages. The low-level context and full-precision card fields carry an explicit `_x10` suffix; public result bonuses remain percentage values. A nonzero fractional leader bonus cannot trigger the legacy zero-entry default.

Exact tiers compare the evaluated bonus itself, not the rounded half-percent ranking key. A 4.8%, 4.9%, 5.1% or 5.2% deck does not hit a request for 5%. Main-card components are summed in integer tenths before display conversion. Limited-count events enumerate tier-observable assignments; cultivation variants and assignments compete independently within each requested tier. The independent `ExactOracle::search_bonus_targets` enumerates these feasible sets directly rather than filtering the overall Bonus winners.

The end-to-end exactness argument and counterexample regressions are recorded in [exactness-proof.md](exactness-proof.md). Formal proofs for every rule that removes search space are collected in [pruning-proof.md](pruning-proof.md). Heuristics are permitted only for incumbent seeding or visit order; they never remove candidates from the exact search frontier.

Exactness is conditional on `SearchCompletion::Complete`. On timeout, every returned incumbent is still legal and exactly evaluated, but canonical Top-K completeness is unproven; `SearchStats::deadline_hit` is set and completion is `TimedOut`. Pools above 512 retain their full candidate set in SoA columns; the fixed metadata-mask accessors explicitly return `None`. A pool larger than 65,535 cards returns `TooManyCards` rather than silently producing an approximate deck.

Compact representation limits are checked before pool packing. The current model uses 16-bit dense indexes and public card IDs, 18-bit per-card power profiles, 8-bit skill values, 12-bit main-card bonus totals in tenths, 15 distinct nonzero limited-bonus values, and 255 distinct entries in each special-skill table. A real event skill cap is applied before the width check. Identical special-skill content is interned, so duplicate entries do not consume distinct capacity. Unrepresentable values return a typed `BuildError::CapacityExceeded`; they are not saturated, truncated, silently removed, or allowed to panic in the arena builder.

Result ordering uses one total order for every solver: objective descending (only minimizing `power` reverses this field), then actual resolved and capped total power descending for `mysekai`, then the ascending sorted public card-ID set, then the ascending concrete legal input-slot card IDs, and finally the prepared-pool variant ordinals. The last field selects a deterministic cultivation representative within the same immutable pool; rebuilding a different pool does not promise the same dense ordinals. Distinct means a public card-ID set, not a slot permutation or cultivation variant. A fixed or forced leader remains a role constraint; the display order materialized by the evaluator is not fed back as a new Specific input order. Exact bonus requests deduplicate separately in each tier.

This canonical ordering replaces the historical traversal-dependent tie representatives and the Mysekai metadata-upper-bound tiebreak. Challenge live decks are always five cards of one character, for every target including `power`/`skill`.

No-event `score` exactness cost: without an event, the builder keeps every representable hard-filtered candidate; it does not use a bonus-blind quality trim. Search combines the scenario-aware suffix bound with the role-aware correlated power/skill bound. The latter is an admissible relaxation of the coupled score formula and is guarded by exhaustive prefix-vs-completion property tests. Special-skill resolution (unit-count / different-unit / reference skills) is still performed only at exact leaves; bounds use per-card maxima, so unresolved coupling can make the relaxation loose but cannot make it underestimate a completion.
