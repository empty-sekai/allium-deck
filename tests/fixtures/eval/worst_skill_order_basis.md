# worst_skill_order basis

Sources:
- moe: sekai-deck-recommend-moe/src/live-score/live-calculator.cpp:124-130 sorts skill rates ascending regardless of Best or Worst.
- C++: sekai-deck-recommend-cpp/src/live-score/live-calculator.cpp:88-110 sorts the first five skill slots and keeps the leader repeat slot out of that sort.
- B: staging/evaluator-20260414-002125/src/lib.rs:903-919 has the first-five-slot sorting shape but needed the rate-direction patch.

Manual derivation:
- Five cards have power 1000 each, so total_power is 5000.
- Skill values are 10, 20, 30, 40, 50.
- Solo skill rates are 1, 2, 3, 4, 5, and the leader repeat rate is 0.
- Worst order sorts skills descending but keeps rates ascending:
  - 50 * 1 / 100 = 0.5
  - 40 * 2 / 100 = 0.8
  - 30 * 3 / 100 = 0.9
  - 20 * 4 / 100 = 0.8
  - 10 * 5 / 100 = 0.5
- Total rate is base 1.0 + 3.5 = 4.5.
- live_score is 4.5 * 5000 * 4 = 90000.
- Average comparison is 110000; Best comparison is 130000.
- Therefore Worst < Average < Best.

Bug comparison:
- Before the patch, Worst linked slot and rate swaps, so descending skills also received descending rates: 50*5 + 40*4 + 30*3 + 20*2 + 10*1 = 550 contribution units, live_score 130000.
- After the patch, Worst gives live_score 90000.
