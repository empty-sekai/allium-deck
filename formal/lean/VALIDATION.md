# 验证记录：2026-09-29

本记录对应本目录的数学证明，不是 Rust 全场景精确性或二进制 refinement 证书。对照源代码基线：`5c6dff7387e57384b9b2c989ab4f535cdd74f098`。

## 实际执行

执行环境为 Windows x86-64，Lean `4.24.0`（提交 `797c613eb9b6d4ec95db23e3e00af9ac6657f24b`）、Mathlib `f897ebcf72cd16f89ab4577d0c826cd14afaafc7`、Python `3.14.7`。依赖提交由 `lake-manifest.json` 固定。证明与依赖在独立工作目录中构建；没有修改 Rust 源码或 Cargo 依赖。

```text
python verify.py --self-test

SOURCE CHECK PASSED: 31 files; 12 imported proof modules
COVERAGE open: 16
COVERAGE out_of_scope: 1
COVERAGE partial: 24
COVERAGE proved: 6
STAGE ONE COMPLETE: False
Build completed successfully (3100 jobs).
AXIOM AUDIT PASSED: 404 declarations; 254 theorems;
  allowlist=[propext, Classical.choice, Quot.sound]
NEGATIVE AUDIT TEST PASSED: sorry
NEGATIVE AUDIT TEST PASSED: foreign-axiom
NEGATIVE AUDIT TEST PASSED: unused-local-axiom
NEGATIVE AUDIT TEST PASSED: native-decide
NEGATIVE COVERAGE TEST PASSED: omitted-pruning-mechanism
NEGATIVE COVERAGE TEST PASSED: excluded-mathematical-obligation
VERIFICATION PASSED FOR THE DECLARED SCOPE (see coverage.json)
```

上面的 `3100 jobs` 是包括依赖在内的 Lake 构建任务数，不是证明数量。公理审计的 `254 theorems` 包括 Lean 自动生成的辅助定理。`Allium/` 中实际有 **131 条显式 theorem 声明，12 个模块，共 1,924 行源码**；行数包含定义、证明和注释，不代表覆盖率。

负向测试不是简单搜索文本，而是把临时错误声明注入已导入的 Lean 环境，再要求传递公理审计实际拒绝它们。测试文件放在临时目录，未作为公理加入正式证明库。

`coverage.json` 中引用的每个定理名还通过 Lean 的 `#check` 验证存在。所有证明模块都必须在 `Allium.lean` 中导入，遗漏文件会在构建前失败。

## 未完成部分不会被绿灯隐藏

41 个原有剪枝机制对应 `P01` 至 `P41`，另外六项说明收集器、搜索、超时、枚举、全场景实例化和 Rust refinement 边界。当前有 24 项部分完成、16 项尚未证明。这些项目的规模并不相同，不能将这些数字换算为形式化验证百分比。

已实际运行 `python verify.py --require-complete`，在构建和公理审计通过后以退出码 1 拒绝全量完成声明，明确列出 40 项部分完成/未证明义务。尤其 `S05`——将各场景的评分、枚举和每一个剪枝连接到 `search_exact` 的前提——仍是第一档的未完成工作，不属于可排除的 Rust refinement。

## CI

`.github/workflows/lean.yml` 在 Linux 上运行同一构建与 `--self-test` 检查。工作流的实际运行结果以对应 PR 的 GitHub Actions 日志为准；本记录不把本地成功写成未经执行的远端成功。

本轮本地未重新运行 Rust 单测、穷举矩阵或性能测试，不引用历史测试数量作为本轮结果。仓库原有的 Rust CI 保持不变。
