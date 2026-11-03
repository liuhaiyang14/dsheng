#!/usr/bin/env bash
# package_skill.sh —— 打包 dsheng-agent Skill ZIP（交付物：Skill ZIP）
#
# 产物：skills/dsheng-agent/skill.zip（包含 SKILL.md、reference/、scripts/，
# 不含可执行文件、Provider 凭据或真实业务数据）。
# 依赖：zip 命令（macOS/Windows Git Bash 自带或可安装；无 zip 时按下方手动说明操作）。
set -eu

HERE="$(cd "$(dirname "$0")/.." && pwd)"   # skills/dsheng-agent
OUT="$HERE/skill.zip"

if ! command -v zip >/dev/null 2>&1; then
  cat >&2 <<'EOF'
错误：未找到 zip 命令。手动打包等价命令（在本仓库根目录执行）：

  (cd skills/dsheng-agent && zip -r skill.zip SKILL.md reference scripts \
    -x 'scripts/__pycache__/*' '*/__pycache__/*' '*.DS_Store')

或将 skills/dsheng-agent 目录压缩为 skill.zip（保留 SKILL.md 在 ZIP 根部）。
EOF
  exit 1
fi

cd "$HERE"
rm -f skill.zip
zip -r skill.zip SKILL.md reference scripts \
  -x '*/__pycache__/*' '*.DS_Store' >/dev/null
printf '已生成 %s\n' "$HERE/skill.zip"
unzip -l skill.zip
