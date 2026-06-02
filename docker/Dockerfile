# allium-deck 验证环境
#
# 只依赖 serde/serde_json/thiserror，从零 release 编译 ~30s，增量数秒。
#
#   docker build -f docker/Dockerfile -t allium-deck-dev .
#   docker run --rm allium-deck-dev                          # 跑 lib 单测

FROM rust:1.94-bookworm AS builder

ENV CARGO_TERM_COLOR=always \
    CARGO_INCREMENTAL=0
WORKDIR /build

# 1) 依赖缓存层（空壳 lib）
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src && echo '' > src/lib.rs && cargo build --release 2>/dev/null || true

# 2) 真实源码 + 测试 + 数据
COPY src ./src
COPY tests ./tests
COPY testdata ./testdata
COPY docker/recommend_cli.rs examples/

RUN touch src/lib.rs && cargo build --release --examples

CMD ["cargo", "test", "--release", "--lib"]
