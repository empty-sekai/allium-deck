# allium-deck 容器

[English](./README.en.md) | 简体中文

## 文件

| 文件 | 作用 |
|------|------|
| `Dockerfile` | 引擎验证环境：缓存依赖层 → 编译 + 跑单测 |
| `Dockerfile.wasm-ci` | WASM 发布 builder：Rust 1.94 + wasm-pack + Python 上传 SDK |
| [`../server/Dockerfile`](../server/Dockerfile) | HTTP 服务镜像，见下方「服务镜像」 |

CLI 真身在 `src/bin/recommend_cli.rs`（`cargo install allium-deck` 出来就是 `recommend_cli` 命令），`docker/` 不放源码。

---

# 验证环境

只依赖 `serde / serde_json / thiserror`，从零 release 编译 ~30s，增量数秒。

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
ALLIUM_MASTERDATA_CN=/abs/masterdata_cn \
ALLIUM_MASTERDATA_JP=/abs/masterdata_jp \
ALLIUM_MUSIC_METAS=/abs/music_metas.json \
ALLIUM_TESTDATA=/abs/testdata/real \
  cargo test --release --test e2e_regression
```

## 关键纪律

- 别改 `Cargo.toml` 的依赖，否则 Docker 依赖缓存层失效、重编所有依赖。
- 性能数字必须在 `--release` 下测。
- 重构类改动：改完跑 e2e，输出必须逐字节不变。

---

# 服务镜像

服务本身是一个直接的实现，只让引擎能通过 HTTP 用起来，不是调优过的架构；
它没做的事列在 [`../server/README.md`](../server/README.md#范围)。下面的镜像同理——
一个多阶段构建加一个非 root 的最小运行时，没有别的花样。

构建上下文是仓库根目录（服务按 path 依赖引擎 crate）：

```bash
docker build -f server/Dockerfile -t allium-deck-server .

docker run --rm -p 8080:8080 -v /path/to/data:/data:ro allium-deck-server \
  --masterdata cn=/data/masterdata --music-metas cn=/data/music_metas.json
```

## 三个独立维度

libc、分配器、运行时基底是三个互不绑定的 build arg：

| 参数 | 取值 | 默认 |
|---|---|---|
| `LIBC` | `gnu` \| `musl` | `gnu` |
| `ALLOC` | 空（用 libc 自带） \| `jemalloc` \| `mimalloc` | 空 |
| `RUNTIME` | `distroless` \| `scratch` | `distroless` |

`musl` 是静态链接，可以配 `scratch` 得到一个不含 libc 的镜像；`gnu` 需要 `distroless`
提供运行时。两个分配器都编译 C 代码，空值则是纯 Rust 依赖树。

```bash
# 默认（gnu + libc 自带分配器 + distroless）
docker build -f server/Dockerfile -t allium-deck-server .

# 最小镜像：musl 静态 + scratch
docker build -f server/Dockerfile --build-arg LIBC=musl --build-arg RUNTIME=scratch .

# 最高吞吐：见下表
docker build -f server/Dockerfile --build-arg ALLOC=mimalloc .
```

## 实测

默认值是测出来的。先说结论：**六个组合里只有一个能被这次测量清楚分开**。

| 镜像 | 镜像大小 | A 吞吐 req/s 中位数（范围） | A p99 ms | B p99 ms | RSS 稳态 MiB |
|---|---:|---:|---:|---:|---:|
| musl + libc 分配器 + scratch | 7.71 MB | **35.1**（30–37） | 324 | 510 | **18** |
| musl + jemalloc + scratch | 8.49 MB | 156.1（134–185） | 113 | 470 | 119 |
| musl + mimalloc + scratch | 7.97 MB | 191.8（145–240） | 69 | 439 | 155 |
| **gnu + libc 分配器 + distroless（默认）** | 50.4 MB | 159.0（103–192） | 108 | 453 | 36 |
| gnu + jemalloc + distroless | 51.2 MB | 176.5（161–212） | 72 | 435 | 96 |
| gnu + mimalloc + distroless | 50.7 MB | 218.3（157–232） | 79 | 439 | 146 |

**A**：活动 multi/score 组卡，`{"eventId":1,"eventType":"marathon","liveType":"multi","target":"score","limit":8}`，并发 8。
**B**：World Bloom 终章（`worldBloomFinaleTurn:3` + `attrFilter`），`timeoutMs:200` 让每个样本花同样的 CPU 预算，并发 8；B 的吞吐被这个预算钉死在 16–17 req/s，所以只列 p99。

结论：

- **musl 配 libc 自带的分配器，在这个负载下明显最差**。A 类吞吐中位数 35.1，是其余五个
  （156–218）的 1/4.4 到 1/6.2，取值区间与它们完全不重叠；B 类 p99 也最高。
- **其余五个彼此分不开**。三轮之间它们的区间大量重叠（最宽的一个 103–240），差距小于
  测量本身的波动——**不要从这张表里读它们的名次**。
- **RSS 的差别倒是稳定的**：不带第三方分配器 18–36 MiB，带上 96–155 MiB，差 3–9 倍。
- 差异主要不在搜索上。A 类请求里搜索只占 0.2–0.4 ms，时间大头是把玩家卡组
  （本次 1300 张卡、约 361 KB）反序列化成结构体，那是大量小分配——分配器在这里起作用。
- 默认取 `gnu` + libc 自带分配器：在分不开的那一组里，它依赖树最干净（纯 Rust，无 C 代码）、
  RSS 最低。换成 `ALLOC=mimalloc` 或 `jemalloc` 不会更差，但本表也证明不了它们更好，
  要换就拿自己的流量测。`LIBC=musl RUNTIME=scratch` 的 7.7 MB 镜像和 18 MiB RSS 仍然
  有吸引力，但别把它用在高频小请求上。

### 口径

这些数字只在**同一台机器、同一批测量内**可比，不能当作绝对性能预期：

- 3 轮，每轮内的镜像顺序按固定随机种子打乱；表里是三轮的**中位数**，括号里是范围。
  单轮测量会给出看似精确的排名，但打乱顺序重测时后五个组合的名次会互相翻转，
  因此这里只给中位数与范围。
- 数据是本仓自带的确定性合成集（26 角色 / 1300 卡 / 满配账号），由
  `cargo run --release --manifest-path server/Cargo.toml --bin export-synth-masterdata`
  生成，不含任何游戏数据。
  合成账号比真实账号大，所以 B 类的绝对耗时不代表真实 World Bloom 请求。
- 宿主 Docker Desktop（WSL2），容器限 4 CPU / 4 GiB，`--workers 4`。
- 压测客户端与服务端在同一台机器上，会互相争 CPU。

## 在自己的流量上复测

服务自己就输出所需的分解，不需要额外探针：

```bash
# 每个响应的 timing：queue / 建池 / 搜索 / 总计
curl -s localhost:8080/v1/recommend -d @request.json -H 'content-type: application/json' \
  | jq .timing

# /metrics 里是同一组分段的直方图，外加队列拒绝与 worker panic 计数
curl -s localhost:8080/metrics | grep -E 'alliumdeck_(search|build_pool|queue_wait)_seconds'
```

用自己的真实请求分布和真实并发度跑，对比几个 `ALLOC` 值的吞吐、p99 与 `docker stats`
的 RSS。微基准里的 `malloc`/`free` 成绩对这个负载参考价值有限——真正决定结果的是请求体
解析的分配模式和搜索的时长分布。
