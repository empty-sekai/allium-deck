# allium-deck standalone 验证环境

脱离 allium-scapus 渲染层（skia/freetype，本机编不了、容器首编 ~15min），单独迭代 allium-deck。

## 为什么可行

`crates/allium-deck/Cargo.toml` 只依赖 `serde / serde_json / thiserror`，**无 skia/freetype/tokio**。
实测：从零（含所有依赖）编译 `allium-deck` release = **~30s**；只改 `src/` 增量重编 = **数秒**。
对比渲染层首编 ~15min。所以建议把 allium-deck 的所有改动验证都放这里，不要进 `Dockerfile.verify`。

## 文件

| 文件 | 作用 |
|------|------|
| `Dockerfile` | builder：缓存依赖层 → 编译 + 跑单测。改 `src/` 不重拉依赖。 |
| `standalone.Cargo.toml` | 最小 workspace 根（替掉 allium-deck 的 `workspace = true` 继承）。 |
| `recommend_cli.rs` | 验证 CLI：masterdata+user+params → 推荐卡组 + 分阶段耗时（建池 vs 搜索）。 |

## 用法 A：本机直接跑（最快，推荐日常迭代）

本机有 cargo 1.94 即可，allium-deck 不需要容器：

```bash
# 在 allium-scapus/ 下。单元测试（自包含，无需外部数据）：
cargo test -p allium-deck --lib --release

# 验证 CLI（需要先把 example 放进去，或用 --example 指向 docker/ 的副本）：
mkdir -p crates/allium-deck/examples
cp crates/allium-deck/docker/recommend_cli.rs crates/allium-deck/examples/
cargo run -p allium-deck --example recommend_cli --release -- \
  --masterdata <masterdata_cn 目录> \
  --music-metas <music_metas.json> \
  --user <user.json> \
  --params <params.json>
# 用完删掉临时 example，别提交：rm -r crates/allium-deck/examples
```

输出示例（stderr 是计时，stdout 是结果）：
```
[load] masterdata+music_metas: 120.3ms
[build_pool] 68.5ms  pool=187 张候选卡        ← P3 关注：建池占大头
[search] 4.2ms  leaf=1234 ub_prunes=... ep_explored=...
[total] build+search = 72.7ms
 1. score=12345678     cards=[123, 456, 789, 234, 567]
 ...
```
改完 P3 后，再跑一次看 `[build_pool]` 是否下降；P1/P2 看结果卡组/分数是否变化。

## 用法 B：Docker（CI 风格，环境干净）

```bash
# 在 allium-scapus 仓库根下构建（上下文必须是仓库根：Dockerfile 里 COPY 的路径都以
# crates/allium-deck/ 开头，且要拷 docker/standalone.Cargo.toml）。末尾的 "." 就是上下文。
docker build -f crates/allium-deck/docker/Dockerfile -t allium-deck-dev .

# 跑单元测试（默认 CMD，自包含）：
docker run --rm allium-deck-dev

# 跑 CLI（挂载外部数据）。Git Bash 加 MSYS_NO_PATHCONV=1 避免路径转换：
MSYS_NO_PATHCONV=1 docker run --rm \
  -v /abs/masterdata_cn:/data/md \
  -v /abs/music_metas.json:/data/mm.json \
  -v /abs/user.json:/data/user.json \
  -v /abs/params.json:/data/params.json \
  allium-deck-dev \
  cargo run --release -p allium-deck --example recommend_cli -- \
    --masterdata /data/md --music-metas /data/mm.json \
    --user /data/user.json --params /data/params.json
```

> 构建上下文注意：Dockerfile 里的 `COPY crates/allium-deck/...` 假设上下文是 **allium-scapus 仓库根**。
> 用 `docker build -f crates/allium-deck/docker/Dockerfile <仓库根>` 指定。

## 跑 e2e 回归（对比 moe 金标准）

e2e（`tests/e2e_regression.rs`）需要外部 masterdata + testdata，通过环境变量注入：

```bash
ALLIUM_MASTERDATA_CN=/abs/masterdata_cn \
ALLIUM_MASTERDATA_JP=/abs/masterdata_jp \
ALLIUM_MUSIC_METAS=/abs/music_metas.json \
ALLIUM_TESTDATA=/abs/testdata/real \
  cargo test -p allium-deck --release --test e2e_regression
```

⚠️ 改 P1（WL 支援）前先读 `IMPROVEMENTS.md §3.1`：当前 e2e 的 masterdata 加载器把 WL 支援表硬编码成空，WL 支援**未被覆盖**，必须先补数据和 case 才能验收。

## 关键纪律（来自踩坑经验）

- **别改 `crates/allium-deck/Cargo.toml` 或 `standalone.Cargo.toml` 的依赖**，否则 Docker 依赖缓存层失效、重编全部依赖。新增验证工具放 `examples/`（cargo 自动发现，无需注册 `[[bin]]`）。
- 性能数字必须在 `--release` 下测（profile：opt-level=3 / lto=fat / codegen-units=1）。
- 重构类改动（如 P3 建池索引化）：改完跑 e2e，输出必须**逐字节不变**。
- `recommend_cli.rs` 真身在 `docker/`，验证时拷进 `examples/`，用完删，**不要提交 `examples/`**。
