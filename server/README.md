# allium-deck-server

[English](./README.en.md) | 简体中文

[`allium-deck`](..) 组卡引擎的 HTTP 服务。

引擎本身是一个纯计算库。这个 crate 补上一个网络服务需要的东西：masterdata 常驻内存、
有界的搜索线程池、逐请求的上限，以及能看出每个请求时间花在哪的指标。

不发布到 crates.io。从本仓源码构建，或者直接跑容器。

## 范围

这是一个直接的实现。它让引擎能通过 HTTP 用起来，并带上共享服务需要的那几道护栏，
但**不是调优过的架构，也不打算做成一套完整的部署方案**。它刻意没做的事：

- **没有 TLS、没有鉴权、没有按调用方的配额。** 只有 `/admin/reload` 有一个静态 token
  守着。其余一律交给前面的反向代理或网关。
- **没有缓存。** 每个请求都重新建池、重新搜索，哪怕输入刚刚见过。
- **不复用已解析的玩家数据。** 每次都从请求体里重新解析一遍卡组。卡组一大，这次解析
  才是常规请求的时间大头，而不是搜索——数据见 [`docker/README.md`](../docker/README.md)。
- **没有取消。** 客户端断开不会停下搜索，线程会把活干完再把结果丢掉。唯一的约束是
  逐请求的超时预算。
- **一个请求一次搜索，单线程。** 请求之间是并发的，但单次搜索不会铺到多个核上。
- **单进程。** 实例之间没有任何协调，各自持有一份 masterdata 和自己的队列。

这些每一条都值得补，但现在都不在。

## 快速开始

本仓不携带任何游戏数据。想先跑起来看看，可以导出基准测试用的那套确定性合成数据——
26 角色、1300 张卡，外加一个满配账号：

```bash
cargo run --release --manifest-path server/Cargo.toml --bin export_synth_masterdata -- ./synth

cd server
cargo run --release -- \
  --masterdata synth=../synth/masterdata \
  --music-metas synth=../synth/music_metas.json
```

然后要一副卡组。`user` 接受玩家卡组的对象形式或其 JSON 原文，`params` 就是引擎自己的
参数契约，字段见 [`docs/parameters.md`](../docs/parameters.md)：

```bash
curl localhost:8080/v1/recommend -H 'content-type: application/json' -d "{
  \"user\": $(cat ../synth/user.json),
  \"params\": {\"liveType\": \"multi\", \"target\": \"score\", \"eventId\": 1, \"limit\": 5}
}"
```

```json
{
  "region": "synth",
  "decks": [
    {
      "rank": 1,
      "targetValue": 3659312905881,
      "cards": [{ "cardId": 80, "powerTotal": 35810, "eventBonus": 25.0, "skillScoreUp": 80.0 }],
      "totalPower": 175929,
      "liveScore": 769689,
      "eventPoint": 852
    }
  ],
  "diagnostics": { "poolSize": 156, "effectiveLiveType": "multi", "leafNodes": 145 },
  "timing": { "queueWaitMs": 0.03, "buildPoolMs": 1.12, "searchMs": 0.38, "totalMs": 10.49 },
  "timedOut": false
}
```

换成真实数据时，`--masterdata` 指向平铺着 masterdata `*.json` 的目录，`--music-metas`
指向 `music_metas.json`。仓库只约定输入格式，不绑定具体数据来源。

## 端点

| 方法 | 路径 | 用途 |
| --- | --- | --- |
| POST | `/v1/recommend` | 组卡。覆盖全部 target 与 live type，含 World Bloom 章节与终章。 |
| POST | `/v1/recommend/challenge-all` | 26 个角色各自的最优挑战卡组，带排名。 |
| POST | `/v1/world-bloom/support-cards` | World Bloom 章节的逐卡支援加成。 |
| POST | `/v1/music/recommend` | 对一副已定卡组，给全部曲目与难度打分排序。 |
| POST | `/v1/live/exact-score` | 给定综合力与技能，按谱面逐 note 计算。 |
| POST | `/v1/area-items/recommend` | 区域道具升级按每硬币提升的综合力排序。 |
| GET | `/v1/regions` | 已载入的区服、表行数，以及生效中的上限。 |
| POST | `/admin/reload` | 从磁盘重新读 masterdata。需要 `--admin-token`。 |
| GET | `/healthz` `/readyz` | 存活与就绪。 |
| GET | `/metrics` | Prometheus 文本格式。 |
| GET | `/openapi.json` | 完整的请求与响应 schema。 |

请求用 `"region"` 指定区服；省略则用默认区服——除非 `--default-region` 另有指定，
否则就是第一个 `--masterdata` 给的那个。

## 配置

每个 `--some-name` 都同时读 `ALLIUM_DECK_SOME_NAME`，命令行优先。

| 参数 | 默认 | 说明 |
| --- | --- | --- |
| `--masterdata <region>=<dir>` | 必填 | 可重复。环境变量里用逗号分隔。 |
| `--music-metas <region>=<file>` | 必填 | 可重复，每个区服一个。 |
| `--bind <addr>` | `0.0.0.0:8080` | |
| `--default-region <name>` | 第一个 `--masterdata` | 请求省略 `region` 时用它。 |
| `--workers <n>` | 可用并行度 | 搜索线程数。 |
| `--max-queue <n>` | `workers * 8` | 排队请求数上限，超出回 503。 |
| `--queue-timeout-ms <ms>` | `1000` | 请求在队列里最多等多久，超出回 504。 |
| `--max-search-timeout-ms <ms>` | `2000` | 请求 `timeoutMs` 的上限。 |
| `--max-limit <n>` | `30` | 请求 `limit` 的上限。 |
| `--max-body-bytes <n>` | `8388608` | 请求体大小上限。 |
| `--admin-token <token>` | 未设置 | 设了才开放 `POST /admin/reload`。 |
| `--log-format <text\|json>` | `text` | 日志级别用 `ALLIUM_DECK_LOG`，如 `debug`。 |

## 并发与背压

组卡搜索是 CPU 密集的，World Bloom 与终章请求会跑上几百毫秒。把这些放在异步运行时上
会把连接处理饿死，所以服务用固定数量的专用线程，前面挡一条有界队列：

```text
HTTP（异步）  --try_send-->  队列（--max-queue）  -->  --workers 个搜索线程
                 满了：503                              一次一个请求
```

- **队列满** → 立刻 `503` 并带 `Retry-After`，而不是把请求吸进一条无界积压里。调用方
  由此知道服务已经饱和。
- **在队列里等超过 `--queue-timeout-ms` 仍未开始** → `504`。
- 一旦线程开始处理就会做完；从那之后由搜索自己的超时预算约束。
- 请求里 panic 只会变成它自己的 `500`，线程继续服务下一个，不会让配置的并发度悄悄缩水。

## 请求上限

引擎允许 `limit` 到 100、`timeoutMs` 到 300000——后者足够让一个请求占住一个线程五分钟。
服务不为此拒绝请求，而是把两者压到自己的上限：要得更多就返回更小的结果，而不是报错。
生效中的上限由 `GET /v1/regions` 给出。

搜索一旦走到超时，会返回当时已找到的最优卡组，响应里 `"timedOut": true`。这些卡组**不是**
被证明的最优解——精确性矩阵见 [`docs/parameters.md`](../docs/parameters.md)。大卡组上的
World Bloom 终章请求最容易撞到这里；如果你的部署宁可多等也不要近似解，把
`--max-search-timeout-ms` 调高。

## 错误

所有失败都是同一个形状，客户端可以直接按 `code` 分支：

```json
{ "error": { "code": "overloaded", "message": "the search queue is full; retry shortly" } }
```

| 状态码 | `code` | 含义 |
| --- | --- | --- |
| 400 | `invalid_request` | 请求体不合法，或参数被引擎拒绝。 |
| 404 | `unknown_region` | 该区服没有载入 masterdata。 |
| 503 | `overloaded` | 队列满。可重试。 |
| 504 | `queue_timeout` | 排队时间超过预算。 |
| 500 | `internal` | 搜索线程失败。 |

## 重新载入 masterdata

```bash
curl -X POST localhost:8080/admin/reload -H "authorization: Bearer $TOKEN"
```

所有配置的区服都会重读，快照整体原子替换。已经在处理中的请求仍持有它开始时的那份快照；
载入失败则保留当前正在用的快照。

## 可观测性

`/metrics` 里有按端点与结果分类的请求计数，以及总耗时**和各阶段各自**的直方图——
排队、建池、搜索。分段才是有用的部分：它说明一个慢请求慢在排队、建池还是搜索上，
而这正是给自己的流量定 `--workers` 与 `--max-queue` 所需要的依据。

同一组分段也在每个响应的 `timing` 对象里。

## 容器

```bash
# 在仓库根目录执行：构建上下文要包含引擎 crate
docker build -f server/Dockerfile -t allium-deck-server .

docker run --rm -p 8080:8080 -v /path/to/data:/data:ro allium-deck-server \
  --masterdata cn=/data/masterdata --music-metas cn=/data/music_metas.json
```

libc、分配器、运行时基底是三个独立的 build arg。各种组合、实测数字，以及怎么在自己的
流量上复测，见 [`docker/README.md`](../docker/README.md)。

运行时镜像里没有 shell，所以健康检查归跑容器的那一层：存活探 `/healthz`，就绪探 `/readyz`。

## 测试

```bash
cargo test --release
```

集成测试在进程内驱动路由，跑在合成 masterdata 上，并要求卡组序列与 `engine::recommend`
逐项一致、顺序也一致——HTTP 这一层不允许改变引擎给出的答案。
