# wl_bonus basis

Sources:
- TS: sekai-calculator/src/event-point/card-bloom-event-calculator.ts, WorldBloom different attribute bonus.
- moe: sekai-deck-recommend-moe/src/deck-information/deck-calculator.cpp:70-86, support deck count accumulation.
- C++: sekai-deck-recommend-cpp/src/event-point/card-bloom-event-calculator.cpp, WorldBloom bonus logic.
- B: staging/evaluator-20260414-002125/src/lib.rs:562-623, diff attr and support deck split.

Manual derivation:
- Five cards each have base event bonus 10, so card total is 50.
- The deck has five distinct attrs, table[5] is 4, so diff_attr_bonus_rate is 4.
- event_bonus_rate is card bonus plus diff attr: 50 + 4 = 54.
- support_deck_count is 12. The first 12 support cards have bonus 1 and none are in the main deck, so support_deck_bonus_rate is 12.

Consistency:
- Three reference implementations use WorldBloom support cards and different attribute bonus. B keeps diff_attr_bonus_rate as an independent output; this task adopts B.
