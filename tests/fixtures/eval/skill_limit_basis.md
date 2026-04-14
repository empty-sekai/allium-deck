# skill_limit basis

Sources:
- TS: sekai-calculator/src/event-point/event-service.ts:125-131, eventSkillScoreUpLimits lookup.
- moe: sekai-deck-recommend-moe/src/card-information/card-skill-calculator.cpp, score-up clamping.
- C++: sekai-deck-recommend-cpp/src/card-information/card-skill-calculator.cpp:10-21, min(scoreUp, limit).
- B: staging/evaluator-20260414-002125/src/lib.rs:665-728, prepare skill candidate selection.

Manual derivation:
- Every card has base score_up 400.
- Limited event passes skill_score_up_limit 300, so output score_up is 300.
- Challenge comparison passes no limit, so output score_up remains 400.

Consistency:
- The old C++ path hardcoded a final chapter limit in caller code. This task adopts the masterdata-driven limit passed through EventContext.
