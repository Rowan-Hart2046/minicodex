# MiniCodex

一个面向 macOS 的最小可运行 coding agent。它使用 Rust 和 OpenAI Responses API，保留 Codex 最核心的代理循环：理解任务、检查项目、调用本地工具、修改文件、运行命令、继续推理并返回结果。

> 这是教学型最小实现，不是 OpenAI Codex 的替代品，也不隶属于 OpenAI。

## 功能

- 交互式连续对话，或单次命令模式
- 列目录、读文件、写文件、执行 zsh 命令
- 文件访问限制在指定工作目录中
- 默认在执行命令前请求确认
- 支持自定义模型和 OpenAI 兼容的 API 地址

## macOS 安装

需要 Rust 1.85+、macOS 自带的 `curl` 和 OpenAI API Key。项目没有第三方 Rust 依赖。

```bash
git clone https://github.com/Rowan-Hart2046/minicodex.git
cd minicodex
cargo build --release
export OPENAI_API_KEY="你的 API Key"
./target/release/minicodex
```

也可以直接运行：

```bash
cargo run -- "检查这个项目并告诉我如何运行"
cargo run -- --workspace /path/to/project "修复测试"
```

默认模型是 `gpt-6-sol`。可按账户权限切换：

```bash
export MINICODEX_MODEL="gpt-6-luna"
cargo run
```

若使用兼容服务，可设置：

```bash
export OPENAI_BASE_URL="https://api.openai.com/v1"
```

## 安全说明

文件工具无法访问工作目录之外的路径。Shell 命令能力很强，默认每次执行前都会询问；仅在可信项目和任务中使用 `--full-auto`。

## 与完整 Codex 的差异

MiniCodex 只实现单代理、本地文件/命令工具与简单会话状态。它没有完整 Codex 的沙箱、流式界面、上下文压缩、MCP、技能、多代理、云任务及精细审批策略。

## 开发

```bash
cargo fmt --check
cargo test
cargo clippy -- -D warnings
```
