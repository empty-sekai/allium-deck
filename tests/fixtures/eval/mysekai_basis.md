# mysekai basis

Sources:
- TS: no matching internal truncation output.
- moe: sekai-deck-recommend-moe/src/mysekai-information/mysekai-event-calculator.cpp, Mysekai event point shape.
- C++: sekai-deck-recommend-cpp/src/mysekai-information/mysekai-event-calculator.cpp, Mysekai event point shape.
- A: staging/evaluator-20260414-002059/src/lib.rs:1281-1287, internal truncation.
- B: staging/evaluator-20260414-002125/src/lib.rs:371 and 384-385, total bonus includes support bonus and permutation value differs from target_value.

Manual derivation:
- Deck power is 5000.
- Card event bonus is 5 * 10 = 50 and support bonus is 2, so total bonus is 52.
- power_bonus is floor((1 + 5000 / 450000) * 10 + eps) / 10 = 1.0.
- event_bonus is floor(52 + eps) / 100 = 0.52.
- segmented point is floor(1.0 * 1.52 + eps) * 500 = 500.
- internal is int(1.0 * 1.52 * 500) = 760.
- Mysekai mode sets live_score and event_point to 0 and forces chosen_mask to 0.
- target_value for ScoreTarget::Mysekai is internal, so 760.

Consistency:
- Internal truncation follows A. total_bonus follows B and includes support deck bonus.
