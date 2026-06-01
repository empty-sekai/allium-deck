# score_multi_ev basis

Sources:
- TS: sekai-calculator/src/live-score/live-calculator.ts, equivalent live-score formula.
- moe: sekai-deck-recommend-moe/src/live-score/live-calculator.cpp, equivalent live-score formula.
- C++: sekai-deck-recommend-cpp/src/live-score/live-calculator.cpp:159-173, active bonus only when `liveType == isMulti(liveType)`.
- B: staging/evaluator-20260414-002125/src/lib.rs:819-968, Rust staging formula.

Manual derivation:
- Five cards have total power 1000, honor 0, so deck power is 5000.
- Music base rate is 1.0, all skill coefficients are 0.
- Multi power_sum is 5 * 5000 = 25000.
- Multi active bonus is 5 * 0.015 * 25000 = 1875.
- Multi live_score is (1.0 * 5000 * 4) + 1875 = 21875.
- Multi event base is 110 + int(21875 / 17000) + min(int(87500 / 340000), 13) = 111.
- event_rate, deck_rate, boost_rate are all 1.0, so event_point is 111.
- Cheerful comparison uses effective Cheerful, active bonus is 0, so live_score is 20000.

Consistency:
- C++/moe/TS agree that Multi gets active bonus and Cheerful does not in this parity target. Adopted C++/moe/B behavior.
