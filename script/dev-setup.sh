#!/bin/bash
# 新设备 clone 后运行：设置 git hooks 路径和开发配置
set -e

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

echo "=== opencode fork 开发环境配置 ==="

# 1. Git hooks（从 .githooks/ 加载）
git config core.hooksPath .githooks
echo "✅ core.hooksPath = .githooks"

# 2. 添加上游（如果还没配）
if ! git remote | grep -q upstream; then
  git remote add upstream https://github.com/anomalyco/opencode.git
  echo "✅ upstream remote 已添加"
else
  echo "⏭️  upstream remote 已存在"
fi

# 3. 检查 bun
if command -v bun &>/dev/null; then
  echo "✅ bun $(bun --version)"
else
  echo "⚠️  bun 未安装，编译需要 bun >= 1.3.14"
fi

echo ""
echo "=== 设置完成 ==="
echo "开发分支:  git checkout dev-ai"
echo "发布流程:  见 .githooks/commit-msg 提示"
