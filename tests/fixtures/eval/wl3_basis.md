# wl3 basis

Sources:
- TS: sekai-calculator/src/deck-information/deck-calculator.ts:73-90, deck power folding.
- moe: sekai-deck-recommend-moe/src/deck-information/deck-calculator.cpp:175-178, WL3 total power cap.
- C++: sekai-deck-recommend-cpp/src/deck-information/deck-calculator.cpp, deck power folding.
- B: staging/evaluator-20260414-002125/src/lib.rs:313-315 and task ruling for `world_bloom.power_total_cap`.

Manual derivation:
- Five cards have total power 100000 each, raw deck power is 500000.
- WorldBloom context passes `power_total_cap = 336000`.
- Evaluator applies the cap after folding, so total_power is 336000.
- support_deck_count is 25. The first 25 support cards have bonus 1 and none are selected, so support_deck_bonus_rate is 25.

Consistency:
- The WL3 cap exists only in moe extension. The evaluator reads the cap from context rather than hardcoding the value.
