# challenge_auto basis

Sources:
- TS: sekai-calculator/src/event-point/event-calculator.ts, Challenge event point formula.
- moe: sekai-deck-recommend-moe/src/event-point/event-calculator.cpp, Challenge formula.
- C++: sekai-deck-recommend-cpp/src/event-point/event-calculator.cpp:10-16, challenge branch.
- B: staging/evaluator-20260414-002125/src/lib.rs:979-982, ChallengeAuto handled with Challenge.

Manual derivation:
- The test keeps base_score and base_score_auto equal, so Challenge and ChallengeAuto produce the same live_score 20000.
- Challenge formula is (100 + self_score / 20000) * 120.
- (100 + 20000 / 20000) * 120 = 12120.

Consistency:
- ChallengeAuto handling is a B-instance ruling and is adopted explicitly.
