# best_skill_order basis

Sources:
- moe: sekai-deck-recommend-moe/src/live-score/live-calculator.cpp:124-130 sorts skill rates ascending whenever skill slots are sorted.
- C++: sekai-deck-recommend-cpp/src/live-score/live-calculator.cpp:88-110 sorts the first five skill slots and keeps the leader repeat slot out of that sort.
- B: staging/evaluator-20260414-002125/src/lib.rs:903-919 contains the same first-five-slot sort shape.

Manual derivation:
- Five cards have power 1000 each, so total_power is 5000.
- Skill values are 10, 20, 30, 40, 50.
- Solo skill rates are 1, 2, 3, 4, 5, and the leader repeat rate is 0.
- Best order pairs small skills with small rates and large skills with large rates:
  - 10 * 1 / 100 = 0.1
  - 20 * 2 / 100 = 0.4
  - 30 * 3 / 100 = 0.9
  - 40 * 4 / 100 = 1.6
  - 50 * 5 / 100 = 2.5
- Total rate is base 1.0 + 5.5 = 6.5.
- live_score is 6.5 * 5000 * 4 = 130000.
- Average comparison: average skill is 30, skill contribution is 30 * (1+2+3+4+5) / 100 = 4.5, live_score is 110000.

Consistency:
- The first five slots are sorted; slot 5 is the leader repeat and is not sorted.
