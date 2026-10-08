# 第二轮：具体 Power 场景与共享阈值证明

日期：2026-09-29。继续 Draft PR #44 的 `formal/lean-core-20260929` 分支；本轮起点是 `769f704deeb291ad1c124bc2dcd3c24afb9cf434`。对照 Rust 仍为 `5c6dff7387e57384b9b2c989ab4f535cdd74f098`，31 个对照文件的哈希未改动。

**结论：首个具体 Power 数学实例已经闭合；第一档全量证明仍未完成。** 没有修改 Rust 搜索或运行时依赖，没有合并 main，也没有把剩余第一档义务改名为 Rust refinement。

## 这次推进了什么

新增 5 个模块、41 条显式定理、784 行 `Allium/*.lean` 源码。现在共有 17 个证明模块、172 条显式定理、2,708 行模块源码。数量不代表完成百分比；自动生成的辅助定理另由公理审计计数。

### 具体 Power 主定理

[`ConcretePower.power_search_exact`](Allium/ConcretePower.lean) 证明：对 `Enumeration.Roles` 定义的五个有序槽位、公开卡牌去重、可选角色去重和队长条件，49 个场景的数学搜索输出与 `Enumeration.exhaustiveKeys` 的 canonical Top-K 完全相同。

这个实例的目标值实际定义为：

```text
每张卡：扫描它携带的单位
        → 用单位 profile、是否全队同单位、是否全队同属性选择八槽表项
        → 取该卡扫描结果的最大值
全队：五张卡求和 + honor
      → 可选 total-power cap
```

八槽表允许任意非负数值，不假设“人数/属性加成越多，power 必然越高”。`PowerModel.count_five_iff_common` 和 `attribute_count_five` 把五人成员计数判断连接到单位交集及同属性条件。

主定理不接受 `SearchTree.Sound`、`score ≤ upper` 或“场景已经完整覆盖”作为外部前提。它在内部证明：

- 所有合法卡组至少属于一个真实场景，且不会被该场景的接纳过滤排除。
- 每个真实 member key 都被其场景的 key 集覆盖；由此直接推导 power 上界。
- 角色去重开启时使用 per-character top-five 上界，关闭时使用 per-card top-five 上界；上界不是通过计算全部完整卡组的最高分取得的。
- honor 与 cap 保持上界，共享 tracker 的严格阈值保留 canonical Top-K；K=0、同分、跨场景相同公共卡组和养成/摆位变体由同一框架处理。

初始种子的唯一额外要求是属于合法候选集合。空种子自动满足它。

**不能扩大这个结论：** `ConcretePower` 显式构造 admitted 合法叶子列表，证明的是带场景级剪枝的数学实现，不是所有生产 DFS 优化，也不提供性能结论。`Enumeration.Roles` 与全部受理域/特殊固定角色语义的连接，以及其他评分目标，仍在 S05 的分母内。

### 不把场景接纳与场景归属混为一谈

[`Composition`](Allium/Composition.lean) 定义了两个不同关系：`Admits` 是卡牌层面的宽松池过滤，`Matches` 是完整卡组的真实场景归属。

生产场景可能访问额外的合法卡组。对非单调查表，不能要求该场景的上界支配所有这些额外卡组，否则证明前提本身就不对。新增 [`ScenarioSearch.RequiredSound`](Allium/ScenarioSearch.lean) 只要求上界支配本场景负责的叶子，同时要求所有实际访问叶子全局合法。

`required_search_spec` 直接复用原有 `Allium.search`，证明它保留 required 候选的覆盖；`runScenes_exact` 再证明多个重叠场景共享 tracker 后的全局 Top-K 精确性。种子或阈值见证不需要属于当前场景，也不假定外部阈值下每个场景还会输出自己的局部 Top-K。

这不是报告一个已确认的 Rust 错误，而是补上此前通用搜索定理不足以直接描述的数学边界。

### Power 上下界与专用场景 envelope

[`PowerModel`](Allium/PowerModel.lean) 证明八槽最大值上界、非空单位集合下的最小值下界、selected/free 组合，以及先加 honor 再应用 cap 的最大化 `< threshold` 和最小化 `> threshold` 剪枝。

[`ScenarioPower`](Allium/ScenarioPower.lean) 单独建模 `solver/power.rs::scenario_power`：空公共单位集合和单单位时精确；多个公共单位时，对单单位假设取最大值得到可靠上界。这个 envelope 与 `Composition.powerBound` 不是同一个公式。

三个静态反例均通过普通 `decide` 生成内核检查的证明，不使用 `native_decide`：

1. 多单位真实值为 20，但 singleton envelope 为 100：它只能作上界，不能替代叶子回算。
2. 卡牌被 Mixed 场景接纳，其 Mixed 上界为 1，但同单位同属性上下文的真实值为 100：接纳不是上界可靠性。
3. 空单位扫描返回 0，而人为给定的八个表项都为 1：低界不能无条件删除非空单位前提。

第三项是解码后数学模型的边界反例，不足以单独断言这个表能由受理输入/压缩构造产生。该构造连接仍明确保留在 P36，未据此声称发现可触发的生产 bug。

## 实际验证

Windows 使用固定 Lean `v4.24.0`、固定 Mathlib 提交 `f897ebcf72cd16f89ab4577d0c826cd14afaafc7` 和 Python 3.14.7。

```sh
cd formal/lean
python verify.py --self-test
```

本轮实际返回 exit code 0：

```text
SOURCE CHECK PASSED: 31 files; 17 imported proof modules
COVERAGE open: 13
COVERAGE out_of_scope: 1
COVERAGE partial: 27
COVERAGE proved: 6
STAGE ONE COMPLETE: False
Build completed successfully (3105 jobs).
AXIOM AUDIT PASSED: 591 declarations; 332 theorems
NEGATIVE AUDIT TEST PASSED: sorry
NEGATIVE AUDIT TEST PASSED: foreign-axiom
NEGATIVE AUDIT TEST PASSED: unused-local-axiom
NEGATIVE AUDIT TEST PASSED: native-decide
NEGATIVE COVERAGE TEST PASSED: omitted-pruning-mechanism
NEGATIVE COVERAGE TEST PASSED: excluded-mathematical-obligation
VERIFICATION PASSED FOR THE DECLARED SCOPE (see coverage.json)
```

公理允许列表仍只有 `propext`、`Classical.choice`、`Quot.sound`。检查器、允许列表、负向测试、对照哈希和 CI 判定逻辑均未放宽。`git diff --check` 通过。

`python verify.py --require-complete` 已实际运行并返回 exit code 1；构建和公理审计通过后，它明确列出 40 项 `partial/open` 义务并拒绝全量完成声明。远端 Linux 与 commitlint 的状态应按本轮提交的 GitHub Actions 结果检查，不能用上一轮的绿灯代替。

## 覆盖变化与尚未闭合的工作

`coverage.json` 保留全部 41 个原始剪枝机制与原顺序。P18、P38、S05 从 `open` 改为 `partial`，P36 的已证明内容和剩余条件细化；没有把任何整项提前标成 `proved`。

因此当前是 **6 proved / 27 partial / 13 open / 1 out_of_scope**。40 个第一档未闭合条目数量未变；它们粒度不同，本轮完成的是其中实质性的子义务，不能按行数换算完成率。

下一批数学连接已经具体到：专用 Power 的 unit-mask 交集工作表完整性、`best_completion` 扫描、least/cap 守卫下的同分公共集合剪枝；numeric 受理域和摘要/固定槽对应；普通/WL/Final 各目标的完整 RegimePlan 特征与评分表达式。支配逆恢复、bonus-tier/refill/certificate、Final/WL 其余剪枝、对数与区间余项、binary64 N1–N7 预算也都继续保留。

这份记录只证明上述已检查的声明，不宣称当前 Rust 二进制、WASM 或全部组卡模式已经形式化验证。
