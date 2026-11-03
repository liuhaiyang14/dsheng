#!/bin/sh
# dsheng 独立 CLI 用户级安装辅助（macOS，用例 37）。
#
# 行为约定（docs/specs/provider-config-delivery.md §6）：
#   - 复制 dsheng 到 ~/.local/bin（默认；DSHENG_INSTALL_DIR 可覆盖）
#   - 不提权（root 运行直接拒绝）、不修改 PATH、不移除 quarantine/绕过 Gatekeeper
#   - 复制前按 SHA256SUMS.txt 校验（存在时）；复制后再校验一次
#
# 用法（在解压后的 CLI 归档目录内）：
#   ./install/macos-user-install.sh                 # 安装 ../dsheng
#   ./install/macos-user-install.sh --from /path/dsheng
#   ./install/macos-user-install.sh --no-verify     # 跳过安装后 --version 冒烟
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
FROM=""
NO_VERIFY=0
while [ $# -gt 0 ]; do
  case "$1" in
    --from) FROM=$2; shift 2 ;;
    --no-verify) NO_VERIFY=1; shift ;;
    -h|--help) sed -n '2,12p' "$0"; exit 0 ;;
    *) echo "未知参数：$1（--help 查看用法）" >&2; exit 2 ;;
  esac
done

if [ "$(id -u)" = "0" ]; then
  echo "错误：请勿用 root/sudo 运行——这是用户级安装（目标 ~/.local/bin）。" >&2
  exit 1
fi

# 定位二进制：--from > 归档内 ../dsheng > 归档内 ../bin/dsheng
if [ -z "$FROM" ]; then
  for cand in "$SCRIPT_DIR/../dsheng" "$SCRIPT_DIR/../bin/dsheng"; do
    if [ -f "$cand" ]; then FROM=$cand; break; fi
  done
fi
[ -n "$FROM" ] || { echo "错误：未找到 dsheng 二进制（--from 指定路径）。" >&2; exit 1; }
FROM=$(CDPATH= cd -- "$(dirname -- "$FROM")" && pwd)/$(basename -- "$FROM")

DEST_DIR=${DSHENG_INSTALL_DIR:-"$HOME/.local/bin"}
DEST="$DEST_DIR/dsheng"

# 安装前校验（SHA256SUMS.txt 存在且包含该二进制时）。
sums=$(find "$SCRIPT_DIR/.." -maxdepth 1 -name SHA256SUMS.txt 2>/dev/null | head -1 || true)
if [ -n "$sums" ]; then
  rel=$(basename -- "$FROM")
  expected=$(grep -E "[ /]$rel\$" "$sums" | awk '{print $1}' || true)
  if [ -n "$expected" ]; then
    actual=$(shasum -a 256 "$FROM" | awk '{print $1}')
    [ "$actual" = "$expected" ] || { echo "错误：SHA-256 校验失败（${rel}）。" >&2; exit 1; }
    echo "校验通过：$rel ($actual)"
  fi
fi

mkdir -p "$DEST_DIR"
cp -- "$FROM" "$DEST"
chmod u+x "$DEST"
echo "已安装：$DEST"

# 安装后冒烟（不绕过 Gatekeeper：若系统拦截属正常安全流程）。
if [ "$NO_VERIFY" -eq 0 ]; then
  if "$DEST" --version >/dev/null 2>&1; then
    echo "冒烟通过：$DEST --version"
  else
    echo "警告：$DEST --version 未成功。" >&2
    echo "若为 Gatekeeper 拦截（首次运行未签名二进制），请按系统提示处理后重试；" >&2
    echo "说明见 docs/release/UNSIGNED.md。本脚本不做任何绕过。" >&2
    exit 1
  fi
fi

echo
echo "PATH 未修改。如需在任意目录调用 dsheng，请自行把下面一行加入 ~/.zshrc（手动，不代改）："
printf '  export PATH="%s:\$PATH"\n' "$DEST_DIR"
echo "然后运行：$DEST --help"
