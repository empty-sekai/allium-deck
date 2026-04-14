# no_event basis

Sources:
- TS: sekai-calculator/src/live-score/live-calculator.ts, solo live score formula.
- moe: sekai-deck-recommend-moe/src/live-score/live-calculator.cpp, solo live score formula.
- C++: sekai-deck-recommend-cpp/src/live-score/live-calculator.cpp:159-173, live score formula.
- B: staging/evaluator-20260414-002125/src/lib.rs:365-369, no event uses live_score as event_point.

Manual derivation:
- Deck power is 5000.
- Solo base rate is 1.0 and skill coefficients are all 0.
- live_score is 1.0 * 5000 * 4 = 20000.
- No event context exists, so event_point equals live_score.
- diff_attr_bonus_rate is 0.

Consistency:
- Three references agree on solo score; B defines no-event output behavior for this evaluator.
