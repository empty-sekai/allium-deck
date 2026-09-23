# CNB 云原生开发

本仓库使用 CNB 标准云原生开发 Workspace 作为按需 Linux 开发机。Workspace 的代码根目录是 `/workspace`，当前项目直接作为主仓库使用。可通过标准 OpenSSH 执行命令，并用 Mutagen 同步本地源码改动。

## Workspace 配置

根目录 [`.cnb.yml`](../.cnb.yml) 使用 CNB 标准 `vscode` service 创建支持 SSH 的 Workspace。

配置申请 8 vCPU。性能测量前，在 Workspace 内记录 CPU 型号、指令集和可用内存。

## SSH 连接

从 CNB 仓库页面的「云原生开发」启动当前分支，在创建成功页面点击右下角的「SSH 登录命令」，复制当前 Workspace 的完整命令。也可从头像 →「我的云原生开发」找回正在运行的实例。Workspace 用户名按实例生成，每次重建都要更新本机 SSH 别名，不能写死或猜测。首次连接时校验 [CNB 官方 host key](https://docs.cnb.cool/en/workspaces/fingerprint.html)：

```text
SHA256:fnWZvpqd+VAIRJxaZdV1KVMFfDgcCjYrP2VSWQ68T/E
```

例如在本机 SSH 配置追加独立别名；`<CNB 页面提供的完整用户名>` 不含 `@cnb.space`：

```sshconfig
Host cnb-allium-deck
    HostName cnb.space
    User <CNB 页面提供的完整用户名>
    Port 22
    StrictHostKeyChecking yes
    ServerAliveInterval 30
    ServerAliveCountMax 3
```

连接后确认：

```bash
pwd
cd /workspace
git status --short --branch
uname -a
lscpu
nproc
free -h
df -h /workspace
```

`/workspace` 必须是当前仓库及分支。不要用 `StrictHostKeyChecking=no` 绕过指纹校验，也不要把临时 Workspace 用户名、令牌或私钥提交到仓库。

## Mutagen 源码同步

本地工作树是同步源，`/workspace` 是构建端。先在 SSH 上确认工作区是预期仓库且没有需要保留的远端源码改动，再建立仓库专属会话：

```powershell
mutagen sync create --name allium-deck-cnb --mode one-way-replica `
  --ignore-vcs --ignore target --ignore .tmp --ignore .worktrees `
  --ignore .codegraph --ignore dist --ignore release --ignore pkg `
  --ignore '.env' --ignore '.env.*' --ignore '*.pem' `
  C:\path\to\allium-deck cnb-allium-deck:/workspace
mutagen sync flush allium-deck-cnb
mutagen sync list allium-deck-cnb
```

该模式会以本地文件为准覆盖远端对应文件；在本地编辑或提交源码，远端运行编译、测试和性能实验。`.git` 由 CNB 的主仓库持有，`target/` 和 `.tmp/` 留在远端作临时产物。Workspace 被回收后，需要从新页面获取 SSH 用户名，更新别名，再重新连接 Mutagen。

## 开发工具链

标准 CNB 开发镜像若没有 Rust 与 C 编译器，在 SSH 中执行：

```bash
cd /workspace
bash scripts/cnb-bootstrap.sh
. /root/.cargo/env
```

脚本只安装项目所需的 C 编译依赖与 Rust 1.98.0、rustfmt、clippy。Workspace 重建后工具链可能不保留，按需重跑。

## Rust 检查与构建

在 `/workspace` 的 SSH shell 执行：

```bash
rustc -Vv
cargo -V
cargo fmt --all --check
cargo clippy --all-features --all-targets -- -D warnings
cargo test --all-features
cargo build --release --bin recommend_cli
```

服务 crate 是独立 Cargo workspace：

```bash
cd /workspace
cargo fmt --manifest-path server/Cargo.toml -- --check
cargo clippy --manifest-path server/Cargo.toml --all-targets -- -D warnings
cargo clippy --manifest-path server/Cargo.toml --all-targets --features jemalloc -- -D warnings
cargo clippy --manifest-path server/Cargo.toml --all-targets --features mimalloc -- -D warnings
cargo test --manifest-path server/Cargo.toml --release
```

WASM 检查按需执行；首次使用时准备 `wasm32-unknown-unknown` target：

```bash
rustup target add wasm32-unknown-unknown
cargo fmt --manifest-path wasm/Cargo.toml -- --check
cargo clippy --manifest-path wasm/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path wasm/Cargo.toml --all-targets --release
```

构建产物和合成数据放在 `.tmp/` 或 `target/`，不要提交大缓存或生成的 `wasm/pkg/`。

## 持久化边界

CNB 会备份 `/workspace` 主仓库中的源码和未提交修改；`target/`、大缓存、被 `.gitignore` 忽略的文件和子 Git 仓库修改不作为可靠持久化边界。需要保留的代码按正常流程提交并推送到远端分支。
