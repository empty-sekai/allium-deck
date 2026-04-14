# custom_mixed basis

Sources:
- TS: no equivalent custom mixed bonus branch.
- moe: sekai-deck-recommend-moe/src/event-point/card-event-calculator.cpp:49-75, custom mixed bonus and supportUnit none pass-through.
- C++: no equivalent custom mixed bonus branch.
- B: staging/evaluator-20260414-002125/src/lib.rs:630-663, custom_bonus_value.

Manual derivation:
- Card 0 matches character 1 and attr Cool, so bonus is 50.
- Card 1 does not match character but matches attr Cool, so bonus is 25.
- Card 2 is character 21, has support_unit None, and the custom support constraint is Idol. support_unit None passes, so character-only bonus is 25.
- event_id is 0 and custom bonus still applies.

Consistency:
- This is moe-only behavior. Adopted moe semantics per task.
