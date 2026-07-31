# allium-deck 验证环境

只依赖 `serde / serde_json / thiserror`，从零 release 编译 ~30s，增量数秒。

## 文件

| 文件 | 作用 |
|------|------|
| `Dockerfile` | builder：缓存依赖层 → 编译 + 跑单测 |
| `Dockerfile.wasm-ci` | CNB WASM 发布 builder：Rust 1.94 + wasm-pack + Python 上传 SDK |

CLI 真身已迁到 `src/bin/recommend_cli.rs`（`cargo install allium-deck` 出来就是 `recommend_cli` 命令），`docker/` 不再放源码。

## 用法 A：本机直接跑

```bash
# 单元测试（自包含，无需外部数据）：
cargo test --lib --release

# CLI（真身在 src/bin/recommend_cli.rs）：
cargo run --bin recommend_cli --release -- \
  --masterdata <masterdata 目录> \
  --music-metas <music_metas.json> \
  --user <user.json> \
  --params <params.json>
```

输出示例（stderr 是计时，stdout 是结果）：
```
[load] masterdata+music_metas: 120.3ms
[build_pool] 68.5ms  pool=187 张候选卡
[search] 4.2ms  leaf=1234 ub_prunes=...
[total] build+search = 72.7ms
 1. score=12345678     cards=[123, 456, 789, 234, 567]
 ...
```

## 用法 B：Docker

```bash
docker build -f docker/Dockerfile -t allium-deck-dev .

# 跑单元测试（默认 CMD，自包含）：
docker run --rm allium-deck-dev

# 跑 CLI（挂载外部数据）：
MSYS_NO_PATHCONV=1 docker run --rm \
  -v /abs/masterdata:/data/md \
  -v /abs/music_metas.json:/data/mm.json \
  -v /abs/user.json:/data/user.json \
  -v /abs/params.json:/data/params.json \
  allium-deck-dev \
  cargo run --release --bin recommend_cli -- \
    --masterdata /data/md --music-metas /data/mm.json \
    --user /data/user.json --params /data/params.json
```

## 跑 e2e 回归

e2e（`tests/e2e_regression.rs`）需要外部 masterdata + testdata，通过环境变量注入：

```bash
ALLIUM_MASTERDATA_CN=<workspace>/local/masterdata/cn \
ALLIUM_MASTERDATA_JP=/abs/masterdata_jp \
ALLIUM_MUSIC_METAS=/abs/music_metas.json \
ALLIUM_TESTDATA=/abs/testdata/real \
  cargo test --release --test e2e_regression
```

## 关键纪律

- 别改 `Cargo.toml` 的依赖，否则 Docker 依赖缓存层失效、重编所有依赖。
- 性能数字必须在 `--release` 下测。
- 重构类改动：改完跑 e2e，输出必须逐字节不变。
