# Business-rule sources

## C++ reference

The JP7 business rules were ported from
**[Team-Haruki/sekai-deck-recommend-cpp](https://github.com/Team-Haruki/sekai-deck-recommend-cpp)**,
using commit
[`496caed78a07ffe1f2d1f1553047dbf6b2ed0314`](https://github.com/Team-Haruki/sekai-deck-recommend-cpp/commit/496caed78a07ffe1f2d1f1553047dbf6b2ed0314)
as the reference. The rules are implemented in allium-deck's Rust data model and
exact-search architecture.

| Business rule | Reference implementation |
| --- | --- |
| Three-dimensional area effects, ALL_MATCH and multi-unit selection | [card-power-calculator.cpp](https://github.com/Team-Haruki/sekai-deck-recommend-cpp/blob/496caed78a07ffe1f2d1f1553047dbf6b2ed0314/src/card-information/card-power-calculator.cpp), [area-item-service.cpp](https://github.com/Team-Haruki/sekai-deck-recommend-cpp/blob/496caed78a07ffe1f2d1f1553047dbf6b2ed0314/src/area-item-information/area-item-service.cpp) |
| Multi-unit activation and original-unit shuffle counting | [deck-calculator.cpp](https://github.com/Team-Haruki/sekai-deck-recommend-cpp/blob/496caed78a07ffe1f2d1f1553047dbf6b2ed0314/src/deck-information/deck-calculator.cpp) |
| MySEKAI gate selection and missing gate-level rows | [card-power-calculator.cpp](https://github.com/Team-Haruki/sekai-deck-recommend-cpp/blob/496caed78a07ffe1f2d1f1553047dbf6b2ed0314/src/card-information/card-power-calculator.cpp), [PR #9](https://github.com/Team-Haruki/sekai-deck-recommend-cpp/pull/9) |
| Finale identification from master data, event limits and default shuffle rates | [master-data.cpp](https://github.com/Team-Haruki/sekai-deck-recommend-cpp/blob/496caed78a07ffe1f2d1f1553047dbf6b2ed0314/src/data-provider/master-data.cpp) |
| Area-item upgrade evaluation | [area-item-recommend.cpp](https://github.com/Team-Haruki/sekai-deck-recommend-cpp/blob/496caed78a07ffe1f2d1f1553047dbf6b2ed0314/src/area-item-recommend/area-item-recommend.cpp) |

Credit also belongs to the earlier
[StarMoe-org/sekai-deck-recommend-cpp](https://github.com/StarMoe-org/sekai-deck-recommend-cpp)
and [NeuraXmy/sekai-deck-recommend-cpp](https://github.com/NeuraXmy/sekai-deck-recommend-cpp)
projects acknowledged by the Haruki fork. The C++ repository publishes its source
under [LGPL-2.1](https://github.com/Team-Haruki/sekai-deck-recommend-cpp/blob/496caed78a07ffe1f2d1f1553047dbf6b2ed0314/LICENSE).

This source map identifies the provenance of the business rules. It does not
claim that the two engines have identical searches or outputs. The Rust engine
retains its independent fixes for other-unit skill counting, static reference
skill values, fractional reference effects, and order-independent averaging.
Its completion and canonical Top-K contracts are documented in
[parameters.md](parameters.md) and [search-validation.md](search-validation.md).

The reference uses fixed 10/30/50 percent shuffle rates for third-round finales.
allium-deck uses those defaults when the selected event has no explicit shuffle
rows. Matching `eventShuffleUnitBonuses` rows replace the defaults as a whole;
an omitted unit count then contributes zero. A real finale requires its skill
cap row; only legacy and simulated finales use the legacy fallback cap.

## Browser-consumer behavior

[Moesekai's published WASM patch series](https://github.com/StarMoe-org/Moesekai/tree/d0e7cd4cd5cdea25756945806f001dfdfdb3111e/web/vendor/allium-deck-wasm/patches)
also provided concrete integration cases: real finale events, raw skill caps,
master-table shuffle bonuses, mandatory cards and characters, and explicit versus automatic
leader selection. Its [vendor documentation](https://github.com/StarMoe-org/Moesekai/blob/d0e7cd4cd5cdea25756945806f001dfdfdb3111e/web/vendor/allium-deck-wasm/README.md)
records those fixes. Consumer membership requirements are distinct from the
engine's default ordered-slot constraints.
