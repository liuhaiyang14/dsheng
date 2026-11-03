#!/usr/bin/env bash
# 发布前检查（S4，用例 30）：secrets/真实业务文件扫描 + 版本清单核对。
#
# 用法：
#   scripts/package/preflight.sh                       # 扫描 git 跟踪文件 + 核对最新清单
#   scripts/package/preflight.sh --dist-dir <产物目录>  # 指定清单产物查找目录
#   scripts/package/preflight.sh --skip-manifest       # 跳过清单核对（仅内容扫描）
#
# 检查项（主计划 §11.1：同步公开仓库/发布前必过）：
#   1) git 跟踪文件中禁止真实业务/数据/凭据文件：
#      .xlsx/.xls/.xlsm/.pptx/.ppt/.sqlite/.sqlite3/.db*/.key/.pem/.p12/.pfx/
#      .env*、exports/、data/、安装包与二进制（.dmg/.msi/.exe 等）
#   2) 文本内容扫描：
#      - 本机绝对路径（/Users/<用户>、/home/<用户>/、C:\Users\<用户>）
#      - 有效 Key 样式（sk-…、ghp_…、github_pat_…、AKIA…、AIza…、xox…、
#        glpat…、PEM PRIVATE KEY 块、Bearer 长串、Z.ai 风格 hex32.id16、
#        key/secret/token/password 赋值 ≥32 字符）
#      行尾标注 `# preflight-allow` 可豁免单行（须写明理由）。
#   3) 版本清单核对：docs/release/manifest-v*.json（最新一份）按哈希复验
#      （调用 version_manifest.py --verify）。
#   4) 大文件守卫：跟踪文件 >3MiB 报错；.zip >1MiB 报错（防产物入库）。
# 退出码：0 通过；1 存在必须修复项。
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_ROOT"

DIST_ARGS=()
SKIP_MANIFEST=0
while [ $# -gt 0 ]; do
  case "$1" in
    --dist-dir) DIST_ARGS+=(--dist-dir "$2"); shift 2 ;;
    --skip-manifest) SKIP_MANIFEST=1; shift ;;
    *) echo "preflight.sh: 未知参数 $1" >&2; exit 2 ;;
  esac
done

FAIL=0
WARN=0
fail() { printf '\033[1;31m[FAIL]\033[0m %s\n' "$*"; FAIL=$((FAIL+1)); }
warn() { printf '\033[1;33m[WARN]\033[0m %s\n' "$*"; WARN=$((WARN+1)); }
ok()   { printf '\033[1;32m[ OK ]\033[0m %s\n' "$*"; }

command -v git >/dev/null 2>&1 || { echo "git 不可用" >&2; exit 2; }

# ---------------------------------------------------------- 1. 禁止文件（按名）
log() { printf '\033[1;34m[preflight]\033[0m %s\n' "$*"; }
log "1/4 禁止文件扫描（git 跟踪清单）"
BAD_EXT_RE='\.(xlsx|xls|xlsm|pptx|ppt|sqlite3?|db|db-wal|db-shm|key|pem|p12|pfx|dmg|msi|exe|dylib|dll|so)$|(^|/)\.env($|\.)|^(exports|data)/'
while IFS= read -r f; do
  if printf '%s' "$f" | grep -Eq "$BAD_EXT_RE"; then
    fail "禁止跟踪的文件：${f}（真实业务/数据/凭据/安装产物）"
  fi
done < <(git ls-files)

ZIP_COUNT=0
while IFS= read -r f; do
  case "$f" in
    *.zip)
      size=$(git cat-file -s "$(git rev-parse ":$f" 2>/dev/null)" 2>/dev/null || echo 0)
      if [ "${size:-0}" -gt 1048576 ]; then
        fail "跟踪的 zip 过大（${size}B > 1MiB）：${f}（产物不入 git，放 target/ 并记入 manifest）"
      else
        warn "跟踪的 zip：${f}（确认非二进制产物）"
      fi
      ZIP_COUNT=$((ZIP_COUNT+1)) ;;
  esac
done < <(git ls-files)
[ "$FAIL" -eq 0 ] && [ "$ZIP_COUNT" -eq 0 ] && ok "无禁止扩展名/数据目录文件"

# ---------------------------------------------------------- 2. 内容扫描
log "2/4 文本内容扫描（本机路径/有效 Key 样式）"
scan_pattern() { # $1=类别 $2=ERE
  local category="$1" pat="$2" hits
  hits="$(git grep -n -I -E -e "$pat" -- . 2>/dev/null | grep -v '# preflight-allow' || true)"
  if [ -n "$hits" ]; then
    while IFS= read -r line; do fail "${category}：$line"; done <<<"$hits"
  fi
}
scan_pattern "本机绝对路径(macOS)" '/Users/[A-Za-z0-9._-]+'
scan_pattern "本机绝对路径(Linux)"  '/home/[A-Za-z0-9._-]+/'
scan_pattern "本机绝对路径(Windows)" 'C:\\Users\\[A-Za-z0-9._-]+'
scan_pattern "本机绝对路径(Windows)" 'C:/Users/[A-Za-z0-9._-]+'
# 注 1：Key 前缀模式用 (^|[^…]) 边界，避免 "task-root" 之类普通词命中 sk- 前缀。
# 注 2：不用 \b——git grep -E 走 POSIX regcomp（macOS/BSD 无 \b），会静默失配。
scan_pattern "Key样式(sk-)"        '(^|[^A-Za-z0-9_-])sk-[A-Za-z0-9_-]{20,}'
scan_pattern "Key样式(github)"     '(^|[^A-Za-z0-9])gh[pousr]_[A-Za-z0-9]{20,}'
scan_pattern "Key样式(github PAT)" '(^|[^A-Za-z0-9])github_pat_[A-Za-z0-9_]{20,}'
scan_pattern "Key样式(AWS AKIA)"   '(^|[^A-Za-z0-9])AKIA[0-9A-Z]{16}'
scan_pattern "Key样式(Google)"     '(^|[^A-Za-z0-9])AIza[0-9A-Za-z_-]{30,}'
scan_pattern "Key样式(Slack)"      '(^|[^A-Za-z0-9])xox[baprs]-[A-Za-z0-9-]{10,}'
scan_pattern "Key样式(gitlab)"     '(^|[^A-Za-z0-9])glpat-[A-Za-z0-9_-]{20,}'
scan_pattern "私钥块"              '-----BEGIN [A-Z ]*PRIVATE KEY-----'
scan_pattern "Key样式(Z.ai hex.id)" '(^|[^A-Za-z0-9])[0-9a-fA-F]{32}\.[A-Za-z0-9]{16}([^A-Za-z0-9]|$)'
scan_pattern "Bearer 长串"          '(^|[^A-Za-z0-9])Bearer [A-Za-z0-9._~+/=-]{20,}'
# 通用赋值：key/secret/token/password 后跟长随机值（≥32，不含下划线，
# 排除 apply_token = apply_token_from_input 这类代码标识符）。
scan_pattern "疑似凭据赋值" \
  '(api[_-]?key|apikey|client_secret|secret|password|passwd|token)["'"'"' ]*[:=][ ]*["'"'"']?[A-Za-z0-9+/=.-]{32,}'
[ "$FAIL" -eq 0 ] && ok "内容扫描无命中"

# ---------------------------------------------------------- 3. 版本清单核对
log "3/4 版本清单核对"
MANIFEST="$(ls -t docs/release/manifest-v*.json 2>/dev/null | head -1 || true)"
if [ "$SKIP_MANIFEST" -eq 1 ]; then
  warn "按参数跳过清单核对"
elif [ -z "$MANIFEST" ]; then
  warn "docs/release/ 下暂无 manifest-v*.json（首次打包前可忽略）"
else
  VER_NOW="$(python3 scripts/package/version_manifest.py --print-version 2>/dev/null || true)"
  case "$(basename "$MANIFEST")" in
    "manifest-v${VER_NOW}.json") ok "清单文件名与当前版本一致：$(basename "$MANIFEST")" ;;
    *) fail "最新清单 $MANIFEST 与当前版本 $VER_NOW 不符（需重新生成）" ;;
  esac
  if python3 scripts/package/version_manifest.py --verify "$MANIFEST" ${DIST_ARGS+"${DIST_ARGS[@]}"} >/tmp/preflight-manifest.log 2>&1; then
    ok "清单哈希核对通过（$(grep -c '^OK' /tmp/preflight-manifest.log) 项）"
  else
    fail "清单哈希核对失败："; sed 's/^/    /' /tmp/preflight-manifest.log >&2
  fi
fi

# ---------------------------------------------------------- 4. 大文件守卫
log "4/4 大文件守卫（>3MiB）"
BIG="$(git ls-files -z | xargs -0 -I{} find {} -maxdepth 0 -size +3145728c -print 2>/dev/null || true)"
if [ -n "$BIG" ]; then
  while IFS= read -r f; do fail "跟踪文件超过 3MiB：$f"; done <<<"$BIG"
else
  ok "无超过 3MiB 的跟踪文件"
fi

# ---------------------------------------------------------- 汇总
echo
if [ "$FAIL" -gt 0 ]; then
  printf '\033[1;31m[preflight] FAIL：%d 项必须修复（警告 %d 项）\033[0m\n' "$FAIL" "$WARN"
  exit 1
fi
printf '\033[1;32m[preflight] PASS\033[0m（警告 %d 项）\n' "$WARN"
exit 0
