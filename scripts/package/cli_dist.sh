#!/usr/bin/env bash
# 独立 CLI 分发归档（S3，用例 37）：release 构建 dsheng → 归档 + 安装辅助 + 校验清单。
#
# 用法：
#   scripts/package/cli_dist.sh                                  # 默认本机 target
#   scripts/package/cli_dist.sh --target aarch64-apple-darwin --target x86_64-apple-darwin
#   环境变量：DIST_DIR（默认 target/release-dist）
#
# 归档内容（dsheng-cli_<ver>_<os>_<arch>.zip）：
#   dsheng(.exe)                       CLI 二进制（与桌面同一 Rust 核心与版本）
#   install/<平台用户级安装辅助脚本>    macOS .sh / Windows .ps1（不提权/不改 PATH/不绕过）
#   README.txt                         安装与 unsigned 说明
#   SHA256SUMS.txt                     成员校验清单
# Windows x64（x86_64-pc-windows-msvc）须在 Windows 宿主（CI）上运行本脚本；
# macOS 上请求该 target 时直接报错说明，不产出未验证产物。
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_ROOT"

TARGETS=()
DIST_DIR="${DIST_DIR:-$REPO_ROOT/target/release-dist}"
SKIP_BUILD=0
while [ $# -gt 0 ]; do
  case "$1" in
    --target) TARGETS+=("$2"); shift 2 ;;
    --dist-dir) DIST_DIR="$2"; shift 2 ;;
    --skip-build) SKIP_BUILD=1; shift ;;
    *) echo "cli_dist.sh: 未知参数 $1" >&2; exit 2 ;;
  esac
done

log() { printf '\033[1;34m[cli-dist]\033[0m %s\n' "$*"; }
die() { printf '\033[1;31m[cli-dist] 错误:\033[0m %s\n' "$*" >&2; exit 1; }

if ! command -v cargo >/dev/null 2>&1 && [ -x "$HOME/.cargo/bin/cargo" ]; then
  export PATH="$HOME/.cargo/bin:$PATH"
fi
command -v cargo >/dev/null 2>&1 || die "cargo 不可用"
command -v python3 >/dev/null 2>&1 || die "python3 不可用"

VERSION="$(python3 scripts/package/version_manifest.py --print-version)" || die "读取版本失败"
OS_NAME="$(uname -s)"

if [ ${#TARGETS[@]} -eq 0 ]; then
  case "$OS_NAME" in
    Darwin) TARGETS=("$(uname -m | sed -e 's/^arm64$/aarch64/' -e 's/^x86_64$/x86_64/')-apple-darwin") ;;
    MINGW*|MSYS*|CYGWIN*) TARGETS=("x86_64-pc-windows-msvc") ;;
    *) die "无法推断本机 target，请用 --target 显式指定" ;;
  esac
fi

mkdir -p "$DIST_DIR"

for TARGET in "${TARGETS[@]}"; do
  case "$TARGET" in
    aarch64-apple-darwin) OS="macos"; ARCH="arm64"; BIN="dsheng"; HELPER="install/macos-user-install.sh" ;;
    x86_64-apple-darwin)  OS="macos"; ARCH="x64";   BIN="dsheng"; HELPER="install/macos-user-install.sh" ;;
    x86_64-pc-windows-msvc)
      OS="windows"; ARCH="x64"; BIN="dsheng.exe"; HELPER="install/windows-user-install.ps1"
      case "$OS_NAME" in
        MINGW*|MSYS*|CYGWIN*) ;;  # Windows 宿主（CI）
        *) die "windows-msvc target 须在 Windows 宿主构建（本机 ${OS_NAME}）；macOS 交叉不产出未验证产物" ;;
      esac ;;
    *) die "不支持的 target：$TARGET" ;;
  esac

  log "== ${TARGET}（$OS/${ARCH}，版本 ${VERSION}）"
  BIN_PATH="$REPO_ROOT/target/$TARGET/release/$BIN"
  if [ "$SKIP_BUILD" -eq 0 ]; then
    log "1/3 cargo build --release -p dsheng-cli --bin dsheng --target $TARGET"
    cargo build --release -p dsheng-cli --bin dsheng --target "$TARGET"
  fi
  [ -f "$BIN_PATH" ] || die "二进制缺失：${BIN_PATH}（交叉构建需 rustup target add ${TARGET}）"

  # 版本/标识断言（本机可运行时）。
  case "$OS" in
    macos)
      BIN_VER="$(python3 -c '
import json,subprocess,sys
out=subprocess.run([sys.argv[1],"--version"],capture_output=True,text=True)
d=json.loads(out.stdout)["data"]
print(d["app_version"],d["identifier"],sep="\t")' "$BIN_PATH")" || die "CLI --version 失败"
      [ "$(printf '%s' "$BIN_VER" | cut -f1)" = "$VERSION" ] || die "CLI 版本不等于 $VERSION"
      [ "$(printf '%s' "$BIN_VER" | cut -f2)" = "com.dsheng.desktop" ] || die "identifier 异常"
      ;;
    *) log "  （Windows 产物本机不可运行，版本断言在 CI/目标机执行——见 RELEASE-CHECKLIST.md）" ;;
  esac

  log "2/3 组装归档"
  NAME="dsheng-cli_${VERSION}_${OS}_${ARCH}"
  STAGE_PARENT="$DIST_DIR/stage"
  STAGE="$STAGE_PARENT/$NAME"
  ARCHIVE="$DIST_DIR/$NAME.zip"
  rm -rf "$STAGE" "$ARCHIVE"
  mkdir -p "$STAGE/install"

  cp "$BIN_PATH" "$STAGE/$BIN"
  [ -f "$REPO_ROOT/$HELPER" ] || die "安装辅助脚本缺失：$REPO_ROOT/$HELPER"
  cp "$REPO_ROOT/$HELPER" "$STAGE/install/$(basename "$HELPER")"
  case "$OS" in macos) chmod +x "$STAGE/install/$(basename "$HELPER")" ;; esac

  cat > "$STAGE/README.txt" <<EOF
dsheng CLI $VERSION — 独立安装（$OS ${ARCH}，unsigned 未签名）
============================================================

内容
  $BIN                 CLI 二进制（与桌面应用同一 Rust 核心和版本）
  install/              用户级安装辅助（不提权、不改 PATH、不绕过安全提示）
  SHA256SUMS.txt        成员 SHA-256

安装（三选一）
  1) macOS：解压后执行  ./install/macos-user-install.sh
     （复制到 ~/.local/bin/dsheng；PATH 提示自行决定是否添加）
  2) Windows：powershell -File .\\install\\windows-user-install.ps1
     （复制到 %LOCALAPPDATA%\\Programs\\dsheng；被策略拦截时按下面手动复制）
  3) 手动：直接把 $BIN 复制到任意用户目录，绝对路径调用即可。

校验
  macOS/Linux：shasum -a 256 -c SHA256SUMS.txt
  Windows：    Get-FileHash -Algorithm SHA256 .\\$BIN

数据目录（与桌面应用共用同一用户库，不依赖解压位置）
  macOS：  ~/Library/Application Support/com.dsheng.desktop/
  Windows：%LOCALAPPDATA%\\com.dsheng.desktop\\

安全提示（不绕过）
  从网络下载的未签名二进制可能触发 Gatekeeper/SmartScreen，属系统正常
  安全流程；处理方式见 docs/release/UNSIGNED.md。本归档不提供绕过脚本。
EOF

  ( cd "$STAGE" && find . -type f ! -name SHA256SUMS.txt | sort | sed 's#^\./##' \
      | xargs shasum -a 256 ) > "$STAGE/SHA256SUMS.txt"

  # 归档内禁止用户数据/业务文件/Key。
  FORBIDDEN_RE='\.(sqlite3?|db|db-wal|db-shm|key|pem|p12|pfx|xlsx|xls|csv|pptx)$|(^|/)\.env$|(^|/)backups/'
  BAD="$( (cd "$STAGE" && find . -type f) | grep -E "$FORBIDDEN_RE" || true)"
  [ -z "$BAD" ] || die "归档内发现禁止文件：$BAD"

  ( cd "$STAGE_PARENT" && zip -q -X -r "$ARCHIVE" "$NAME" ) || die "zip 失败"

  ZL="$(zipinfo -1 "$ARCHIVE")"
  echo "$ZL" | grep -q "^$NAME/$BIN\$" || die "zip 内缺 $BIN"
  echo "$ZL" | grep -q "^$NAME/install/$(basename "$HELPER")\$" || die "zip 内缺安装辅助"
  echo "$ZL" | grep -q "^$NAME/SHA256SUMS.txt\$" || die "zip 内缺校验清单"

  log "3/3 完成：$ARCHIVE"
  rm -rf "$STAGE"
  printf '  %s  %s\n' "$(shasum -a 256 "$ARCHIVE" | cut -c1-64)" "$NAME.zip"
done

rmdir "$DIST_DIR/stage" 2>/dev/null || true
log "下一步：python3 scripts/package/version_manifest.py --dist-dir \"$DIST_DIR\""
