#!/usr/bin/env bash
# macOS 桌面打包（S2，用例 12/18）：.app → .dmg + 便携 .zip。
#
# 用法：
#   scripts/package/macos.sh                        # 默认 aarch64-apple-darwin（本机 arm64）
#   scripts/package/macos.sh --target x86_64-apple-darwin   # Intel 交叉（需 rustup target add）
#   环境变量：DIST_DIR（产物目录，默认 target/release-dist）、TAURI_CLI（默认自动探测）
#
# 步骤：
#   1. npm --prefix src-ui build
#   2. cargo build --release -p dsheng-cli --bin dsheng --target <t>（便携包内同版本 CLI）
#   3. cargo tauri build（frontendDist 用 --config 覆盖为 ../src-ui/dist）→ .app/.dmg
#   4. 便携 zip：dsheng.app + dsheng CLI + migrations/ + unsigned 说明 README
#   5. 包内断言：可执行/资源/migration 齐全；无用户数据库/Key/业务文件
#
# 约束：不签名、不绕过 Gatekeeper；产物只落 DIST_DIR（target/ 已 gitignore，
# 大文件不入 git，版本清单由 version_manifest.py 记录哈希）。
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_ROOT"

TARGET="aarch64-apple-darwin"
DIST_DIR="${DIST_DIR:-$REPO_ROOT/target/release-dist}"
SKIP_UI=0 SKIP_TAURI=0 SKIP_CLI=0
while [ $# -gt 0 ]; do
  case "$1" in
    --target) TARGET="$2"; shift 2 ;;
    --dist-dir) DIST_DIR="$2"; shift 2 ;;
    --skip-ui-build) SKIP_UI=1; shift ;;
    --skip-tauri-build) SKIP_TAURI=1; shift ;;
    --skip-cli-build) SKIP_CLI=1; shift ;;
    *) echo "macos.sh: 未知参数 $1" >&2; exit 2 ;;
  esac
done

log() { printf '\033[1;34m[macos]\033[0m %s\n' "$*"; }
die() { printf '\033[1;31m[macos] 错误:\033[0m %s\n' "$*" >&2; exit 1; }

# 确保 rustup/cargo 可用（登录 shell 外运行时 PATH 可能不含 ~/.cargo/bin）。
if ! command -v cargo >/dev/null 2>&1 && [ -x "$HOME/.cargo/bin/cargo" ]; then
  export PATH="$HOME/.cargo/bin:$PATH"
fi
command -v cargo >/dev/null 2>&1 || die "cargo 不可用（需要 rustup stable）"
command -v npm  >/dev/null 2>&1 || die "npm 不可用"

VERSION="$(python3 scripts/package/version_manifest.py --print-version)" || die "读取版本失败"
case "$TARGET" in
  aarch64-apple-darwin) ARCH="arm64" ;;
  x86_64-apple-darwin)  ARCH="x64" ;;
  *) die "不支持的 target：${TARGET}（本脚本仅 macOS）" ;;
esac
export MACOSX_DEPLOYMENT_TARGET=13.0

# Tauri CLI：优先 cargo tauri；否则 npx @tauri-apps/cli（版本 2）。
tauri_cli() {
  if [ -n "${TAURI_CLI:-}" ]; then
    # shellcheck disable=SC2086
    $TAURI_CLI "$@"
  elif cargo tauri --version >/dev/null 2>&1; then
    cargo tauri "$@"
  else
    npx --yes --package @tauri-apps/cli@2 tauri "$@"
  fi
}

log "版本 $VERSION / target $TARGET (macos-$ARCH) / 产物目录 $DIST_DIR"
mkdir -p "$DIST_DIR"

# ---------------------------------------------------------------- 1. 前端构建
if [ "$SKIP_UI" -eq 0 ]; then
  log "1/6 npm --prefix src-ui run build"
  npm --prefix src-ui run build
fi
[ -f src-ui/dist/index.html ] || die "src-ui/dist/index.html 不存在：先构建前端"
# 防呆：确认不是 placeholder（tauri.conf 默认 frontendDist 是占位目录）。
if cmp -s src-ui/dist/index.html src-tauri/placeholder-dist/index.html; then
  die "src-ui/dist/index.html 与 placeholder 相同：前端构建产物可疑"
fi

# ---------------------------------------------------------------- 2. CLI 二进制
CLI_BIN="$REPO_ROOT/target/$TARGET/release/dsheng"
if [ "$SKIP_CLI" -eq 0 ]; then
  log "2/6 cargo build --release -p dsheng-cli --bin dsheng --target $TARGET"
  cargo build --release -p dsheng-cli --bin dsheng --target "$TARGET"
fi
[ -x "$CLI_BIN" ] || die "CLI 二进制缺失：${CLI_BIN}（交叉构建需 rustup target add ${TARGET}）"
CLI_VER="$(python3 -c '
import json,subprocess,sys
out=subprocess.run([sys.argv[1],"--version"],capture_output=True,text=True)
d=json.loads(out.stdout)["data"]
print(d["app_version"],d["identifier"],d["db_schema_version"],sep="\t")' "$CLI_BIN")" || die "CLI --version 失败"
CLI_APP_VER="$(printf '%s' "$CLI_VER" | cut -f1)"
CLI_IDENT="$(printf '%s' "$CLI_VER" | cut -f2)"
CLI_SCHEMA="$(printf '%s' "$CLI_VER" | cut -f3)"
[ "$CLI_APP_VER" = "$VERSION" ] || die "CLI 版本 $CLI_APP_VER != $VERSION"
[ "$CLI_IDENT" = "com.dsheng.desktop" ] || die "CLI identifier 异常：$CLI_IDENT"

# ---------------------------------------------------------------- 3. Tauri 构建
APP_BUNDLE="$REPO_ROOT/src-tauri/target/$TARGET/release/bundle/macos/dsheng.app"
if [ "$SKIP_TAURI" -eq 0 ]; then
  # 说明：tauri CLI 在仓库根（而非 src-tauri）执行 beforeBuildCommand，
  # tauri.conf.json 内的 `npm --prefix ../src-ui` 会指到仓库外。此处用
  # --config 同时覆盖 frontendDist（../src-ui/dist，相对 tauri.conf.json）
  # 与 beforeBuildCommand（绝对路径 --prefix），不改仓库配置文件。
  TAURI_CONFIG="$(printf \
    '{"build":{"frontendDist":"../src-ui/dist","beforeBuildCommand":"npm --prefix \\"%s\\" run build"}}' \
    "$REPO_ROOT/src-ui")"
  log "3/6 tauri build（--config 覆盖 frontendDist/beforeBuildCommand）"
  (cd src-tauri && tauri_cli build --config "$TAURI_CONFIG" --target "$TARGET") \
    || die "tauri build 失败（详见上方输出）"
fi

# ---------------------------------------------------------------- 4. 包内断言（.app）
log "4/6 检查 .app 内容"
[ -d "$APP_BUNDLE" ] || die ".app 不存在：$APP_BUNDLE"
PLIST="$APP_BUNDLE/Contents/Info.plist"
[ -f "$PLIST" ] || die "Info.plist 缺失"
pb() { /usr/libexec/PlistBuddy -c "Print :$1" "$PLIST" 2>/dev/null; }
IDENT="$(pb CFBundleIdentifier)"
EXEC_NAME="$(pb CFBundleExecutable)"
MIN_VER="$(pb LSMinimumSystemVersion)"
BUNDLE_VER="$(pb CFBundleShortVersionString)"
[ "$IDENT" = "com.dsheng.desktop" ] || die "CFBundleIdentifier=$IDENT ≠ com.dsheng.desktop"
[ -x "$APP_BUNDLE/Contents/MacOS/$EXEC_NAME" ] || die "主可执行缺失：Contents/MacOS/$EXEC_NAME"
[ "$MIN_VER" = "13.0" ] || die "LSMinimumSystemVersion=$MIN_VER ≠ 13.0"
[ "$BUNDLE_VER" = "$VERSION" ] || die "CFBundleShortVersionString=$BUNDLE_VER ≠ $VERSION"
# 资源说明：Tauri v2 默认把前端资产嵌入主二进制（无独立 Resources 文件）；
# 无 icon 配置时 bundle 不含图标——此处如实告警（配置属 src-tauri 所有权，
# 报告不改）。前端资产已嵌入的旁证：主二进制显著大于纯壳体积。
APP_BIN_SIZE=$(stat -f%z "$APP_BUNDLE/Contents/MacOS/$EXEC_NAME")
[ "$APP_BIN_SIZE" -gt 5000000 ] || die "主二进制仅 ${APP_BIN_SIZE}B：前端资产疑似未嵌入"
if [ -d "$APP_BUNDLE/Contents/Resources" ]; then
  log "  .app Resources：$(ls "$APP_BUNDLE/Contents/Resources" | tr '\n' ' ')"
else
  log "  .app 无 Contents/Resources（资产嵌入二进制；无图标配置——已作为缺口报告）"
fi
log "  .app OK：$EXEC_NAME / $IDENT / min $MIN_VER / v$BUNDLE_VER / ${APP_BIN_SIZE}B"

# ---------------------------------------------------------------- 5. 便携 zip
log "5/6 组装便携 zip"
STAGE="$DIST_DIR/stage/dsheng_${VERSION}_macos_${ARCH}_portable"
PORTABLE_ZIP="$DIST_DIR/dsheng_${VERSION}_macos_${ARCH}_portable.zip"
rm -rf "$STAGE" "$PORTABLE_ZIP"
mkdir -p "$STAGE/migrations" "$STAGE/bin"

ditto "$APP_BUNDLE" "$STAGE/dsheng.app" || die "复制 .app 失败"
cp "$CLI_BIN" "$STAGE/bin/dsheng"
chmod +x "$STAGE/bin/dsheng"

MIG_DIR="$REPO_ROOT/crates/dsheng-core/migrations"
cp "$MIG_DIR"/*.sql "$STAGE/migrations/"
# migration 清单文件：与 repository MIGRATIONS include_str 嵌入清单核对后写入。
EMBEDDED="$(grep -o 'include_str!("../../migrations/[^"]*")' \
  crates/dsheng-core/src/repository/mod.rs | sed -e 's#.*migrations/##' -e 's#")$##')"
diff <(printf '%s\n' "$EMBEDDED" | sort) <(ls "$MIG_DIR" | sort) >/dev/null \
  || die "migrations 目录与 repository MIGRATIONS 嵌入清单不一致"
{
  echo "dsheng $VERSION 随包 SQLite migration 清单（与 crates/dsheng-core 同源）"
  echo "schema_version=$CLI_SCHEMA"
  echo
  (cd "$STAGE/migrations" && shasum -a 256 *.sql)
} > "$STAGE/migrations/MANIFEST.txt"

cat > "$STAGE/README.txt" <<EOF
dsheng $VERSION — macOS 便携版（${ARCH}，unsigned 未签名）
=========================================================

内容
  dsheng.app/          桌面应用（Tauri 2 壳）
  bin/dsheng           同版本独立 CLI（与桌面共用用户数据库）
  migrations/          SQLite migration 清单（应用首启自动执行，此处供核对）
  CHECKSUMS.txt        成员 SHA-256

首次启动（unsigned，不绕过系统安全检查）
  从网络下载解压后首次双击 dsheng.app 可能被 Gatekeeper 拦截
  （“无法验证开发者”/“无法检查恶意软件”）。处理方式见
  系统设置 → 隐私与安全性 → “仍要打开”，完整说明：
  docs/release/UNSIGNED.md。本包不提供、也不应使用任何绕过脚本。

数据目录（“便携”指免安装，不是数据放程序旁）
  macOS：~/Library/Application Support/com.dsheng.desktop/
  首次运行在此创建 dsheng.sqlite3；卸载/删除本包不删除用户数据。
  本包不含任何用户数据库、Provider 配置或业务文件。
EOF
{
  (cd "$STAGE" && find dsheng.app/Contents/MacOS bin migrations -type f | sort | xargs shasum -a 256)
} > "$STAGE/CHECKSUMS.txt"

# 包内断言：禁止用户数据库/Key/业务文件混入。
FORBIDDEN_RE='\.(sqlite3?|db|db-wal|db-shm|key|pem|p12|pfx|xlsx|xls|csv|pptx)$|(^|/)\.env$|(^|/)backups/'
BAD="$( (cd "$STAGE" && find . -type f) | grep -E "$FORBIDDEN_RE" || true)"
[ -z "$BAD" ] || die "便携包内发现禁止文件（用户数据/Key/业务文件）：$BAD"
# migration 数量与 schema 版本一致。
MIG_COUNT="$(ls "$STAGE/migrations"/*.sql | wc -l | tr -d ' ')"
[ "$MIG_COUNT" = "$CLI_SCHEMA" ] || die "migration 数 $MIG_COUNT ≠ schema 版本 $CLI_SCHEMA"

(cd "$DIST_DIR/stage" && ditto -c -k --keepParent "dsheng_${VERSION}_macos_${ARCH}_portable" "$PORTABLE_ZIP") \
  || die "打包 portable zip 失败"
log "  便携 zip：$PORTABLE_ZIP"

# zip 成员断言（用归档列表复查，防止打包过程混入）。
ZL="$(zipinfo -1 "$PORTABLE_ZIP")"
echo "$ZL" | grep -q "^dsheng_${VERSION}_macos_${ARCH}_portable/dsheng.app/Contents/MacOS/$EXEC_NAME$" \
  || die "zip 内缺桌面可执行"
echo "$ZL" | grep -q "^dsheng_${VERSION}_macos_${ARCH}_portable/bin/dsheng$" \
  || die "zip 内缺 CLI 二进制"
for f in $EMBEDDED; do
  echo "$ZL" | grep -q "^dsheng_${VERSION}_macos_${ARCH}_portable/migrations/$f$" \
    || die "zip 内缺 migration：$f"
done
echo "$ZL" | grep -E "$FORBIDDEN_RE" >/dev/null \
  && die "zip 成员含禁止文件" || true

# ---------------------------------------------------------------- 6. DMG 收集
log "6/6 收集 DMG"
TAURI_DMG="$(ls "$REPO_ROOT"/src-tauri/target/"$TARGET"/release/bundle/dmg/*.dmg 2>/dev/null | head -1 || true)"
[ -n "$TAURI_DMG" ] || die "bundle/dmg 下未找到 .dmg"
DMG_OUT="$DIST_DIR/dsheng_${VERSION}_macos_${ARCH}.dmg"
cp "$TAURI_DMG" "$DMG_OUT"
hdiutil imageinfo "$DMG_OUT" >/dev/null || die "DMG 校验失败（hdiutil imageinfo）"

# ---------------------------------------------------------------- 输出摘要
rm -rf "$DIST_DIR/stage"
log "完成。产物："
for f in "$DMG_OUT" "$PORTABLE_ZIP"; do
  printf '  %s  %s\n' "$(shasum -a 256 "$f" | cut -c1-64)" "$(basename "$f")"
done
log "下一步：python3 scripts/package/version_manifest.py --dist-dir \"$DIST_DIR\""
