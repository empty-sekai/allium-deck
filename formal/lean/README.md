# Allium 的 Lean 数学证明

**状态：第一档部分完成，尚未完成全量验证。** 这里没有对当前 Rust 引擎作出“已经形式化验证”的声明。

代码对照基线为 `5c6dff7387e57384b9b2c989ab4f535cdd74f098`，即合并 PR #43 后的 main。Lean 工程不修改 Rust 搜索、评分、超时、服务器或 WASM 实现，也不改变它们的运行时依赖。

## 已经证明什么

所有下列模块都由 `Allium.lean` 导入并实际编译。它们包含有证明项的定理，不是 `sorry` 占位文件。

| 模块 | 已证明的数学内容 |
| --- | --- |
| `TopK`、`Collection`、`Canonical` | 公共卡组身份与具体养成/摆位身份分离；五字段完整顺序；结果数量为 `min(K, 不同公共卡组数)`；逐次插入截断不依赖插入顺序；局部 Top-K 合并等于全局 Top-K。 |
| `Search`、`Budget` | 一个实际定义的分支限界数学算法；严格上界剪枝；合法种子保留；遗漏叶子的覆盖证书；完整搜索精确；超时只能返回已访问合法候选，超时标志不能被后续分支清除。 |
| `Enumeration` | 五个有序槽位的独立合法性规范与完整有限枚举等价；固定角色位于固定卡牌之后；Final 的固定队长占据第 0 槽；同一卡牌的全部合法养成变体仍在枚举中。 |
| `FiniteBounds` | 不同角色的 Top-r 最大值松弛；可用角色不足的不可行性；独立分量上界；后缀缩小时的单调性；最大化/最小化的重复极值界；压缩上界状态的安全性。 |
| `Arithmetic` | 打包整数的字典序条件；严格阈值的整数除法等价；向上除法；上下界取交；有范围前提的安全截断及可选上界回退；严格误差小于网格间隙时的取整桥梁。 |
| `Skill` | `2 * sum + 8 * leader` 技能键；非单调人数技能表的前缀最大值；异单位数量上界；最大值/最小值/平均值参考技能；未知成员用 cap 放宽；参考规则的逐分量合并。 |
| `Quadratic` | 相关评分包络的两个分支；联合 power/skill 半平面；矩形与半平面相交后乘积峰值的四个分支。 |
| `DynamicProgramming` | 有限组选择的完整可达性；必须组与可跳过组的区别；保持精确 key 的逐分量最大值压缩；每一轮压缩后的覆盖不变量；键区间查询与不可达排除。 |
| `Support` | 有限支援池的最大和；主队排除集合增大时支援不增加；不同队长 profile 的逐 ID 最大值包络；排序前缀的阈值证书；替换损失的截止值代数；单调舍入加法下的逐项支配。 |

这些是数学机制的证明。表中出现一个辅助机制，不代表使用它的整个 Rust 函数、场景或所有前提已经证明。

## 主定理及其边界

`Allium.search_exact` 证明：对满足 `SearchTree.Sound score` 的有限搜索树，合法种子驱动的分支限界结果与全部叶子的 canonical Top-K 完全相同。这里的 `score` 必须和候选完整顺序的主目标一致。

`Allium.complete_exact` 将结论扩展到有中断检查的算法，但要求实际返回 `deadlineHit = false`。`TimedOut` 分支只有合法性结论，不能因此宣称精确 Top-K。

`SearchTree.Sound` **是待实例化的前提，不是已经验证所有生产上界的同义词**。当前尚未把各个 Allium 场景的完整评分、合法完成集合和每一个启用剪枝全部连接到该前提。这个连接仍属于第一档，而不是可以推迟到 Rust refinement 的工作。

同样，`Arithmetic.grid_floor_of_error` 的严格误差前提必须独立证明。当前并没有从 `pruning-proof.md` 第 29 节的全部 binary64 表达式推导出 N1–N7 误差预算；不能把实数或有理数定理当作这一缺口的替代品。

## 逐项覆盖与剩余义务

[`coverage.json`](coverage.json) 按 [`pruning-proof.md` 第 26 节](../../docs/pruning-proof.md#26-proof-to-code-inventory) 原顺序列出全部 **41 个剪枝机制**，另列 canonical collection、数学搜索、超时、槽位枚举、全场景实例化和 Rust refinement 边界。每项给出对照源文件、已存在定理以及仍未证明的内容。

状态含义如下：`proved` 指该条明确陈述的数学机制已经证明；`partial` 指仅完成部分引理；`open` 指尚未证明；`out_of_scope` 仅用于第一档之外的 Rust 实现/编译等价。**覆盖行数和 Lean 定理行数不是“验证百分比”。** 当前 `stage_one_complete` 为 `false`。

尚未闭合的主要内容包括完整支配守卫与 Top-K 逆恢复、49 个 composition regimes 与 Power unit-set 场景、精确 bonus-tier 的 first-N/key/slack/refill/certificate 构造、Final 与 WL 的完整场景连接、对数弦界及向外区间/atanh 余项证明、全部评分目标的浮点误差预算。清单中的每项都保留具体缺口，不用未经证明的假设冒充完成。

Rust 的内存安全、缓存一致性、数据读取、机器指令、编译器、WASM，以及 `RustSearch = LeanSpec` 的 refinement 不属于本轮第一档声明。

## 复现检查

需要 Elan/Lean 和 Python 3.11 或更新版本。工具链固定在 Lean `v4.24.0`；Mathlib 固定到 `f897ebcf72cd16f89ab4577d0c826cd14afaafc7`，其依赖由 `lake-manifest.json` 锁定。

```sh
cd formal/lean
lake exe cache get
python verify.py --self-test
```

验证脚本检查对照源文件的 SHA-256、全部模块导入、覆盖表引用的定理名、整库编译，以及所有 `Allium` 声明的传递公理依赖。Windows 的 CRLF 与 Linux 的 LF 仅作换行归一化，不忽略其他源码变化。

公理允许列表只有 `propext`、`Classical.choice`、`Quot.sound`。负向测试实际注入四类错误并要求检查失败：`sorry`、通过外部辅助声明引入的自定义公理、尚未使用的本地公理、`native_decide` 的求值公理。普通 `decide` 产生可由内核检查的证明，与 `native_decide` 的信任路径不同。

要检查是否达到第一档全量完成，执行：

```sh
python verify.py --require-complete
```

**当前这条命令应当失败并列出未完成义务。** 普通 CI 通过只表示“声明范围内的证明与审计通过”，不表示全部生产搜索已验证。验证脚本还会拒绝从第 26 节清单中删去剪枝机制，或将第一档数学义务改成 `out_of_scope`；这两种情况也有负向测试。

## 修改与信任范围

源文件哈希变化会使验证脚本失败。应先审查差异，修改相关模型/证明并核对覆盖义务，再更新对照哈希。仅重算哈希不能证明新代码正确。

可信基础包括 Lean 内核、标准公理、固定依赖与实际运行的工具链。公理审计能检查证明依赖，不能替代对定理陈述是否对应游戏规则、前提是否充分、模型是否连接到实际场景的审查。本文和覆盖表刻意保留这些区别。
