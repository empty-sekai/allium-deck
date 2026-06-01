# cheerful basis

Sources:
- TS: sekai-calculator/src/event-point/event-calculator.ts, Cheerful event formula.
- moe: sekai-deck-recommend-moe/src/event-point/event-calculator.cpp, Cheerful event formula.
- C++: sekai-deck-recommend-cpp/src/event-point/event-calculator.cpp:24-31, life_rate branch.
- B: staging/evaluator-20260414-002125/src/lib.rs:999-1007, Cheerful life rate and boost order.

Manual derivation:
- Multi live in CheerfulCarnival is precomputed as effective Cheerful.
- Cheerful live_score has no active bonus, so it is 1.0 * 5000 * 4 = 20000.
- base_score is 110 + int(20000 / 17000) + min(int(80000 / 340000), 13) = 111.
- life is 1000, so life_rate is 1.15 + clamp(1000 / 5000, 0.1, 0.2) = 1.35.
- event_point is int(int(111 * 1.0 * 1.0) * 1.35) = 149.

Consistency:
- Three references agree on the Cheerful life-rate branch. Active bonus remains Multi-only.
