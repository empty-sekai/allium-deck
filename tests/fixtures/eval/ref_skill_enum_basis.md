# ref_skill_enum basis

Sources:
- TS: sekai-calculator/src/deck-information/deck-calculator.ts, reference skill handling differs in floor timing.
- moe: sekai-deck-recommend-moe/src/deck-information/deck-calculator.cpp:244-270, per-card floor then choose strategy.
- C++: sekai-deck-recommend-cpp/src/deck-information/deck-calculator.cpp:244-270, same reference strategy logic.
- B: staging/evaluator-20260414-002125/src/lib.rs:730-817, materialize_permutation.

Manual derivation:
- Card 0 has normal skill 100 and Ref candidate base 80, rate 50%, max 100, so enumerate_mask includes bit 0.
- Other cards expose score_up_to_reference values 300, 200, 100, 50.
- Per-card floors are min(floor(value * 0.5), 100): 100, 100, 50, 25.
- Max strategy gives 80 + 100 = 180.
- Min strategy gives 80 + 25 = 105.
- Average strategy gives 80 + (100 + 100 + 50 + 25) / 4 = 148.75.

Consistency:
- TS has a known floor timing difference. The adopted behavior is moe/C++ per-card flooring.
