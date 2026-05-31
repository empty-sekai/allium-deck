# allium-deck standalone — 快速迭代/验证环境
#
# 目标：脱离 allium-scapus 渲染层（skia/freetype，本机编不了、容器首编 ~15min），
# 单独编译 allium-deck（只依赖 serde/serde_json/thiserror），实现秒级改→编→验证。
#
# 关键设计：依赖层与源码层分离。改 src/ 只重编 allium-deck 本体（~数秒），
# 不重新拉取/编译 serde。改 Cargo.toml 才会让依赖层失效。
#
# 用法见同目录 README.md。

# ---- builder：缓存依赖，编译 + 跑测试 ----
FROM rust:1.94-bookworm AS builder

ENV CARGO_TERM_COLOR=always \
    CARGO_INCREMENTAL=0

# 国内镜像加速（按需删）。
RUN sed -i 's|deb.debian.org|mirrors.aliyun.com|g' /etc/apt/sources.list.d/debian.sources || true

WORKDIR /build

# 1) 先只拷 manifest，构造空壳 lib 预热依赖缓存层。
#    docker/standalone.Cargo.toml 是一个最小 workspace 根，把 allium-deck 当成员，
#    覆盖掉 allium-deck 原本依赖父 workspace（serde = { workspace = true }）的问题。
COPY crates/allium-deck/docker/standalone.Cargo.toml ./Cargo.toml
COPY crates/allium-deck/Cargo.toml ./crates/allium-deck/Cargo.toml
RUN mkdir -p crates/allium-deck/src \
    && echo '' > crates/allium-deck/src/lib.rs \
    && cargo build --release -p allium-deck 2>/dev/null || true

# 2) 拷真实源码 + 测试 + 自带 mock 数据，编译并跑单测。
COPY crates/allium-deck/src ./crates/allium-deck/src
COPY crates/allium-deck/tests ./crates/allium-deck/tests
COPY crates/allium-deck/testdata ./crates/allium-deck/testdata
COPY crates/allium-deck/docker/recommend_cli.rs ./crates/allium-deck/examples/recommend_cli.rs

# 触碰 lib.rs 让 cargo 感知源码变化（空壳 mtime 早于真实文件时的保险）。
RUN touch crates/allium-deck/src/lib.rs \
    && cargo build --release -p allium-deck --examples

# 默认入口：跑单元测试（不含需要外部 masterdata 的 e2e）。
CMD ["cargo", "test", "--release", "-p", "allium-deck", "--lib"]
