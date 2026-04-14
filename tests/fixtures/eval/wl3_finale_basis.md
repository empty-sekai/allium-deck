# wl3_finale basis

Sources:
- TS: sekai-calculator/src/event-point/event-service.ts:115 and card event bonus limit masterdata.
- moe: sekai-deck-recommend-moe/src/deck-information/deck-calculator.cpp:28-33, final chapter leader bonus removal.
- C++: sekai-deck-recommend-cpp/src/deck-information/deck-calculator.cpp, final chapter deck bonus behavior.
- B: staging/evaluator-20260414-002125/src/lib.rs:531-557, final chapter card bonus shape.

Manual derivation:
- Each card starts with limited 10, leader honor 2, leader limit 3.
- Leader card 0 gets 10 + 2 + 3 = 15.
- Cards 1 through 3 remove leader-only bonuses and keep limited, so each is 10.
- card_bonus_count_limit is 4, so card 4 loses its limited bonus and contributes 0.
- event_bonus_rate is 15 + 10 + 10 + 10 + 0 = 45.
- best_skill_as_leader is forced false for event id 180, so card 0 remains leader even though card 4 has higher skill.

Consistency:
- The final chapter extension is implemented in moe and captured by B. The evaluator uses context-supplied count limit.
