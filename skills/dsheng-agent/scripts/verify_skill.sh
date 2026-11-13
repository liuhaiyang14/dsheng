#!/usr/bin/env bash
# verify_skill.sh —— dsheng-agent Skill 本地逻辑演练脚本
#
# 层次声明：本脚本是「本地逻辑演练层，非 Provider 调用」。全部生成类命令只走
# --dry-run（本地预览），不携带 --confirm-send，不产生任何真实 HTTP 请求；
# 任务候选为合成夹具注入（等价于仓库 S2 集成测试的 DB 夹具层次，非模型响应）。
#
# 前置：bash、python3（stdlib：zipfile/json/sqlite3）、可定位的 dsheng 可执行文件。
# 演练库/目录全部位于 mktemp 临时目录，结束自动清理，绝不动用户真实数据目录。
# 可重复执行（每次全新临时库）。
#
# 用法：bash skills/dsheng-agent/scripts/verify_skill.sh
#   可用环境变量：DSHENG_BIN（显式指定 dsheng 路径）

set -u

STEPS=0
ASSERTS=0
FAILS=0

step() { STEPS=$((STEPS + 1)); printf '\n[步骤 %02d] %s\n' "$STEPS" "$1"; }
ok_msg() { printf '  ✓ %s\n' "$1"; }
assert() { # assert <描述> <条件表达式结果: 0 成功>
  ASSERTS=$((ASSERTS + 1))
  if [ "$2" -eq 0 ]; then ok_msg "$1"; else FAILS=$((FAILS + 1)); printf '  ✗ %s\n' "$1"; fi
}
expect_rc() { # expect_rc <期望码> <实际码> <描述>
  ASSERTS=$((ASSERTS + 1))
  if [ "$1" -eq "$2" ]; then ok_msg "$3 (exit=$2)"; else FAILS=$((FAILS + 1)); printf '  ✗ %s（期望 exit=%s 实际 %s）\n' "$3" "$1" "$2"; fi
}
field() { # field <stdout-json> <点分路径，数组用数字下标> —— 打印字段值（envelope 内）
  printf '%s' "$1" | DSHENG_PATH="$2" python3 -c '
import json, os, sys
d = json.load(sys.stdin)
for p in os.environ["DSHENG_PATH"].split("."):
    if not p:
        continue
    d = d[int(p)] if isinstance(d, list) else d[p]
print(d if not isinstance(d, bool) else str(d).lower())'
}

# ---------- 定位 dsheng ----------
if [ -n "${DSHENG_BIN:-}" ] && [ -x "$DSHENG_BIN" ]; then
  DSHENG="$DSHENG_BIN"
elif command -v dsheng >/dev/null 2>&1; then
  DSHENG="$(command -v dsheng)"
else
  ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
  for c in "$ROOT/target/release/dsheng" "$ROOT/target/debug/dsheng"; do
    if [ -x "$c" ]; then DSHENG="$c"; break; fi
  done
fi
if [ -z "${DSHENG:-}" ]; then
  echo "错误：找不到 dsheng（设置 DSHENG_BIN 或将其加入 PATH）" >&2
  exit 1
fi
command -v python3 >/dev/null 2>&1 || { echo "错误：需要 python3" >&2; exit 1; }
printf 'dsheng = %s\n本脚本为本地逻辑演练层，非 Provider 调用；全部生成命令仅 --dry-run。\n' "$DSHENG"

# ---------- 临时目录 ----------
TMP="$(mktemp -d "${TMPDIR:-/tmp}/dsheng-skill-XXXXXX")"
cleanup() { rm -rf "$TMP"; }
trap cleanup EXIT
DATA="$TMP/data"
mkdir -p "$DATA"
DB="$TMP/verify.sqlite3"

# ---------- 合成核算表 XLSX（python3 stdlib；sheet 名「核算表」以命中 accounting 锚点） ----------
python3 - "$DATA" <<'PYEOF'
import sys, zipfile, html
out = sys.argv[1]
rows = [
    ["科目", "单位", "月度预定", "月度实绩", "年度计划·当月目标"],
    ["销售额", "元", "12000.5", "11500.25", "12500"],
]
def cell(col, r, text):
    return ('<c r="%s%d" t="inlineStr"><is><t>%s</t></is></c>'
            % (chr(ord("A") + col), r, html.escape(text, quote=False)))
body = "".join('<row r="%d">%s</row>' % (ri, "".join(cell(ci, ri, c) for ci, c in enumerate(row)))
               for ri, row in enumerate(rows, start=1))
sheet = ('<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
         '<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">'
         '<sheetData>%s</sheetData></worksheet>' % body)
ct = ('<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
      '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
      '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
      '<Default Extension="xml" ContentType="application/xml"/>'
      '<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>'
      '<Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>')
rels = ('<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
        '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>')
wb = ('<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
      '<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" '
      'xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">'
      '<sheets><sheet name="核算表" sheetId="1" r:id="rId1"/></sheets></workbook>')
wbrels = ('<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
          '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
          '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>')
with zipfile.ZipFile(out + "/核算表模板.xlsx", "w", zipfile.ZIP_DEFLATED) as z:
    z.writestr("[Content_Types].xml", ct)
    z.writestr("_rels/.rels", rels)
    z.writestr("xl/workbook.xml", wb)
    z.writestr("xl/_rels/workbook.xml.rels", wbrels)
    z.writestr("xl/worksheets/sheet1.xml", sheet)
PYEOF

run() { # run <args...> —— 输出捕获到 $OUT/$RC
  OUT="$("$DSHENG" "$@" 2>"$TMP/stderr.log")"
  RC=$?
}

step "版本契约（dsheng --version --json）"
run --version --json
expect_rc 0 "$RC" "退出码 0"
V=$(field "$OUT" "data.identifier")
assert "envelope ok=true" "$([ "$(field "$OUT" "ok")" = "true" ]; echo $?)"
assert "identifier=com.dsheng.desktop（实际 ${V}）" "$([ "$V" = "com.dsheng.desktop" ]; echo $?)"

step "workspace create（合成企业；显式 --db 临时库）"
cat > "$TMP/ws.json" <<'EOF'
{"schema_version":"1","operation":"workspace_create","name":"Skill演练企业","initial_period_policy":{"cadence":"month","effective_from":"2026-01-01"}}
EOF
run workspace create --db "$DB" --input-json "$TMP/ws.json" --json
expect_rc 0 "$RC" "退出码 0"
WS=$(field "$OUT" "data.workspace.workspace_id")
assert "返回 workspace_id（${WS}）" "$([ -n "$WS" ] && [ "$WS" != "None" ]; echo $?)"
assert "周期默认值输出：timezone_id=Asia/Shanghai" "$([ "$(field "$OUT" "data.initial_period_policy.policy.timezone_id")" = "Asia/Shanghai" ]; echo $?)"
assert "周期默认值输出：week_start_day=1" "$([ "$(field "$OUT" "data.initial_period_policy.policy.week_start_day")" = "1" ]; echo $?)"

step "provider set（占位名+占位 Key，仅经 --api-key-stdin；不发生任何连接/模型调用）"
cat > "$TMP/provider.json" <<EOF
{"schema_version":"1","operation":"provider_upsert","workspace_id":"$WS","provider_id":null,"expected_revision":null,"config":{"name":"演练Provider占位","protocol":"chat_completions","base_url":"https://example.invalid/v1","model":"synthetic-model","stream":false,"timeouts":{"connect_seconds":15,"response_seconds":180,"total_seconds":240},"budget":{"context_window_tokens":65536,"output_reserve_tokens":8192}},"key_action":"replace"}
EOF
OUT="$(printf 'placeholder-key-not-real' | "$DSHENG" provider set --db "$DB" --workspace "$WS" --input-json "$TMP/provider.json" --api-key-stdin --json 2>"$TMP/stderr.log")"
RC=$?
expect_rc 0 "$RC" "退出码 0"
PID=$(field "$OUT" "data.provider.provider_id")
assert "返回 provider_id（${PID}）" "$([ -n "$PID" ] && [ "$PID" != "None" ]; echo $?)"
assert "公开输出仅 key_present=true" "$([ "$(field "$OUT" "data.provider.key_present")" = "true" ]; echo $?)"
assert "stdout 不回显 Key" "$(printf '%s' "$OUT" | grep -q 'placeholder-key-not-real'; [ $? -ne 0 ]; echo $?)"
run provider use --db "$DB" --workspace "$WS" --provider "$PID" --expected-revision 1 --json
expect_rc 0 "$RC" "provider use 设为默认（expected-revision=workspace config_revision=1）"

step "metric set（sales_net 指标定义）"
cat > "$TMP/metric.json" <<'EOF'
{"metric_key":"sales_net","label":"销售额","semantic_type":"flow","unit_signature":{"dimension":"money","base_unit":"元","currency":"CNY","tax_basis":"exclusive"},"direction":"up","target_constraint_scope":"monthly_revenue_floor","revenue_semantics":true,"aggregation":"sum_disjoint","required_inputs":[],"comparison_policy":{"positive_base":"relative_change","zero_or_negative_base":"absolute_only"},"effective_from":"2026-01-01","expected_revision":0}
EOF
run metric set --db "$DB" --workspace "$WS" --input-json "$TMP/metric.json" --json
expect_rc 0 "$RC" "退出码 0"
MID=$(field "$OUT" "data.metric_id")
assert "返回 metric_id（${MID}）" "$([ -n "$MID" ] && [ "$MID" != "None" ]; echo $?)"

step "scan（本地目录扫描，0 次 Provider）"
run scan --db "$DB" --workspace "$WS" --dir "$DATA" --json
expect_rc 0 "$RC" "退出码 0"
assert "文件识别为 mapped（核算表锚点）" "$([ "$(field "$OUT" "data.files.0.mapping_status")" = "mapped" ]; echo $?)"

step "analyze（本地计算；产生 run/revision/指纹）"
run analyze --db "$DB" --workspace "$WS" --dir "$DATA" --from 2026-10-01 --to 2026-10-31 --grain auto --json
expect_rc 0 "$RC" "退出码 0"
RUN=$(field "$OUT" "data.run_id")
REV=$(field "$OUT" "data.current_revision_no")
assert "返回 run_id（${RUN}）" "$([ -n "$RUN" ] && [ "$RUN" != "None" ]; echo $?)"
assert "metrics[0].actual_value 为十进制字符串 11500.25" "$([ "$(field "$OUT" "data.metrics.0.actual_value")" = "11500.25" ]; echo $?)"
assert "mom.availability_status=unavailable（无基期不合成）" "$([ "$(field "$OUT" "data.metrics.0.mom.availability_status")" = "unavailable" ]; echo $?)"
run analysis show --db "$DB" --workspace "$WS" --run "$RUN" --json
expect_rc 0 "$RC" "analysis show 只读查看已存快照"

step "diagnosis generate --dry-run（断言：不发送且返回 confirmation_id）"
run diagnosis generate --db "$DB" --workspace "$WS" --run "$RUN" --framework 5why \
  --problem "销售额低于预定（演练）" --evidence manual:skill-verify-1 --dry-run --json
expect_rc 0 "$RC" "退出码 0"
CID=$(field "$OUT" "data.confirmation_id")
assert "返回 confirmation_id（${CID}）" "$([ -n "$CID" ] && [ "$CID" != "None" ]; echo $?)"
assert "workflow=diagnosis" "$([ "$(field "$OUT" "data.workflow")" = "diagnosis" ]; echo $?)"
assert "request_count_maximum=1" "$([ "$(field "$OUT" "data.request_count_maximum")" = "1" ]; echo $?)"
assert "key_present=true（不输出 Key 本体）" "$([ "$(field "$OUT" "data.key_present")" = "true" ]; echo $?)"
ATTEMPTS=$(python3 -c "import sqlite3;print(sqlite3.connect('$DB').execute('SELECT COUNT(*) FROM generation_attempts').fetchone()[0])")
assert "未创建任何 attempt（generation_attempts=0，实为 ${ATTEMPTS}）——未发生发送" "$([ "$ATTEMPTS" = "0" ]; echo $?)"

step "门禁拒绝路径 1：仅 --confirm-send（缺 confirmation-id）→ INVALID_ARGUMENT/2，不发送"
run task generate --db "$DB" --workspace "$WS" --run "$RUN" --confirm-send --json
expect_rc 2 "$RC" "退出码 2"
assert "错误码 INVALID_ARGUMENT" "$([ "$(field "$OUT" "error.code")" = "INVALID_ARGUMENT" ]; echo $?)"

step "门禁拒绝路径 2：伪 confirmation-id → CONFIRMATION_REQUIRED/2，不发送"
run task generate --db "$DB" --workspace "$WS" --run "$RUN" \
  --confirmation-id 00000000-0000-4000-8000-000000000001 --confirm-send --json
expect_rc 2 "$RC" "退出码 2"
assert "错误码 CONFIRMATION_REQUIRED" "$([ "$(field "$OUT" "error.code")" = "CONFIRMATION_REQUIRED" ]; echo $?)"
ATTEMPTS=$(python3 -c "import sqlite3;print(sqlite3.connect('$DB').execute('SELECT COUNT(*) FROM generation_attempts').fetchone()[0])")
assert "仍无 attempt（未发生发送）" "$([ "$ATTEMPTS" = "0" ]; echo $?)"

step "task generate --dry-run（任务生成预览，不发送）"
run task generate --db "$DB" --workspace "$WS" --run "$RUN" --dry-run --json
expect_rc 0 "$RC" "退出码 0"
TCID=$(field "$OUT" "data.confirmation_id")
assert "返回 confirmation_id（${TCID}）" "$([ -n "$TCID" ] && [ "$TCID" != "None" ]; echo $?)"
assert "workflow=task / operation=task_generate" "$([ "$(field "$OUT" "data.workflow")" = "task" ] && [ "$(field "$OUT" "data.operation")" = "task_generate" ]; echo $?)"

step "注入合成候选夹具（本地 DB 夹具层次，模拟已保存的模型候选；非 Provider 响应）"
CAND_ID=$(python3 - "$DB" "$WS" "$RUN" "$REV" "$MID" <<'PYEOF'
import sys, json, uuid, sqlite3
from datetime import datetime, timezone
db, ws, run, rev, mid = sys.argv[1:6]
cand = {
    "summary": "合成演练候选（本地夹具，非模型输出）",
    "tasks": [{
        "task_key": "t-synth-1", "title": "下月销售额追赶（合成）", "summary": "演练候选",
        "category": "revenue", "metric_id": mid, "target_value": 13000, "unit": "元",
        "owner": "合成责任人", "planned_start": "2026-11-01", "due_date": "2026-11-30",
        "source_period": {"from": "2026-10-01", "to": "2026-10-31"},
        "target_period": {"from": "2026-11-01", "to": "2026-11-30"},
        "steps": [{"step_id": "s1", "order": 1, "action": "加大老客拜访（合成）",
                   "owner": "合成责任人", "start_date": "2026-11-02", "due_date": "2026-11-10",
                   "done_criteria": "完成10家拜访", "evidence_ids": []}],
        "evidence_ids": [], "assumptions": [], "needs_review": []}],
}
conn = sqlite3.connect(db)
fp = conn.execute("SELECT analysis_input_fingerprint FROM analysis_runs WHERE run_id=?", (run,)).fetchone()[0]
cid = str(uuid.uuid4())
now = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
conn.execute(
    "INSERT INTO task_candidates (candidate_generation_id, workspace_id, run_id, analysis_revision_no,"
    " analysis_input_fingerprint, generation_input_fingerprint, candidate_revision_no, attempt_id,"
    " output_json, envelope_locally_reconstructed, state, prompt_version, schema_version, created_at)"
    " VALUES (?,?,?,?,?,?,1,'attempt-fixture',?,0,'candidate','2.3','2',?)",
    (cid, ws, run, int(rev), fp, "gen-" + fp, json.dumps(cand, ensure_ascii=False), now))
conn.commit()
print(cid)
PYEOF
)
assert "合成候选已注入（${CAND_ID}）" "$([ -n "$CAND_ID" ]; echo $?)"

step "task reconcile --dry-run（本地 diff + apply_token，0 次 Provider）"
python3 - "$DB" "$WS" "$RUN" "$CAND_ID" "$TMP/reconcile.json" <<'PYEOF'
import sys, json, uuid, sqlite3
db, ws, run, cid, outp = sys.argv[1:6]
conn = sqlite3.connect(db)
rev, fp = conn.execute("SELECT current_revision_no, analysis_input_fingerprint FROM analysis_runs WHERE run_id=?", (run,)).fetchone()
inp = {"schema_version": "1", "workspace_id": ws, "run_id": run, "analysis_revision_no": rev,
       "input_fingerprint": fp, "candidate_generation_id": cid, "candidate_revision_no": 1,
       "expected_tasks": [], "mutation_id": str(uuid.uuid4()),
       "mappings": [{"candidate_task_key": "t-synth-1", "action": "new"}]}
open(outp, "w").write(json.dumps(inp, ensure_ascii=False))
PYEOF
run task reconcile --db "$DB" --workspace "$WS" --run "$RUN" --input-json "$TMP/reconcile.json" --dry-run --json
expect_rc 0 "$RC" "退出码 0"
TOKEN=$(field "$OUT" "data.apply_token")
assert "返回 apply_token（${TOKEN}）" "$([ -n "$TOKEN" ] && [ "$TOKEN" != "None" ]; echo $?)"
assert "created_new 含 t-synth-1" "$([ "$(field "$OUT" "data.created_new.0")" = "t-synth-1" ]; echo $?)"

step "reconcile --apply 无 apply_token → CONFIRMATION_REQUIRED/2（本地双确认）"
run task reconcile --db "$DB" --workspace "$WS" --run "$RUN" --input-json "$TMP/reconcile.json" --apply --json
expect_rc 2 "$RC" "退出码 2"
assert "错误码 CONFIRMATION_REQUIRED" "$([ "$(field "$OUT" "error.code")" = "CONFIRMATION_REQUIRED" ]; echo $?)"

step "reconcile --apply（输入顶层带 apply_token；仍为本地操作，0 次 Provider）"
python3 - "$TMP/reconcile.json" "$TOKEN" "$TMP/reconcile_apply.json" <<'PYEOF'
import sys, json
inp = json.load(open(sys.argv[1]))
inp["apply_token"] = sys.argv[2]
open(sys.argv[3], "w").write(json.dumps(inp, ensure_ascii=False))
PYEOF
run task reconcile --db "$DB" --workspace "$WS" --run "$RUN" --input-json "$TMP/reconcile_apply.json" --apply --json
expect_rc 0 "$RC" "退出码 0"
NEWTASK=$(field "$OUT" "data.new_task_ids.0")
assert "创建永久任务（${NEWTASK}）" "$([ -n "$NEWTASK" ] && [ "$NEWTASK" != "None" ]; echo $?)"

step "task list / task show（只读）"
run task list --db "$DB" --workspace "$WS" --json
expect_rc 0 "$RC" "task list 退出码 0"
run task show --db "$DB" --workspace "$WS" --task "$NEWTASK" --json
expect_rc 0 "$RC" "task show 退出码 0"
assert "初始 status=not_started" "$([ "$(field "$OUT" "data.status")" = "not_started" ]; echo $?)"
TASK_REV=$(field "$OUT" "data.task_revision_no")

step "task update --patch-json（task-edit progress 操作：manual 40% + adjustment_reason）"
cat > "$TMP/progress.json" <<EOF
{"schema_version":"1","workspace_id":"$WS","task_id":"$NEWTASK","expected_task_revision":$TASK_REV,"mutation_id":"$(python3 -c 'import uuid;print(uuid.uuid4())')","operation":{"kind":"progress","progress_mode":"manual","progress":"40","status":"in_progress","adjustment_reason":"演练：步骤未全完成但总体推进40","blocked_reason":null}}
EOF
run task update --db "$DB" --workspace "$WS" --task "$NEWTASK" --patch-json "$TMP/progress.json" --json
expect_rc 0 "$RC" "退出码 0"
assert "progress=40 / status=in_progress" "$([ "$(field "$OUT" "data.progress")" = "40" ] && [ "$(field "$OUT" "data.status")" = "in_progress" ]; echo $?)"
NEWREV=$(field "$OUT" "data.task_revision_no")

step "task step add（新增一条演练步骤；日期须在任务期间内）"
cat > "$TMP/step.json" <<'EOF'
{"action":"复核追赶方案（演练）","owner":"合成责任人","start_date":null,"due_date":"2026-11-05","done_criteria":"输出复核记录"}
EOF
run task step add --db "$DB" --workspace "$WS" --task "$NEWTASK" --input-json "$TMP/step.json" --json
expect_rc 0 "$RC" "退出码 0"

step "export（bundle/workbook/meeting；本地导出，0 次 Provider）"
run export --db "$DB" --workspace "$WS" --run "$RUN" --bundle business --out "$TMP/bundle.zip" --json
expect_rc 0 "$RC" "export --bundle business"
assert "导出包含业务文件清单" "$([ "$(field "$OUT" "data.kind")" = "bundle" ]; echo $?)"
run export --db "$DB" --workspace "$WS" --run "$RUN" --workbook task --out "$TMP/task.xlsx" --json
expect_rc 0 "$RC" "export --workbook task"
run export --db "$DB" --workspace "$WS" --run "$RUN" --meeting --format md --out "$TMP/meeting.md" --json
expect_rc 0 "$RC" "export --meeting --format md"
run template download --package source --profile generic --out "$TMP/source-template.zip" --json
expect_rc 0 "$RC" "template download --package source --profile generic"
run template download --package output --contract two_tables_v1 --out "$TMP/two-tables-output.zip" --json
expect_rc 0 "$RC" "template download --package output --contract two_tables_v1"

step "history list（版本历史只读）"
run history list --db "$DB" --workspace "$WS" --json
expect_rc 0 "$RC" "退出码 0"
assert "runs[0].current_revision_no=${REV}（重做追加 revision）" "$([ "$(field "$OUT" "data.runs.0.current_revision_no")" = "$REV" ]; echo $?)"

step "全程未发生任何 Provider 请求（attempts=0；本脚本从未携带 --confirm-send）"
ATTEMPTS=$(python3 -c "import sqlite3;print(sqlite3.connect('$DB').execute('SELECT COUNT(*) FROM generation_attempts').fetchone()[0])")
assert "generation_attempts=0（实为 ${ATTEMPTS}）" "$([ "$ATTEMPTS" = "0" ]; echo $?)"

printf '\n========================================\n'
printf '演练完成：步骤 %d，断言 %d，失败 %d\n' "$STEPS" "$ASSERTS" "$FAILS"
printf '层次：本地逻辑演练（全 dry-run/本地命令），未发起任何 Provider 调用。\n'
if [ "$FAILS" -gt 0 ]; then
  printf '存在失败断言，请检查上方 ✗ 条目。\n'
  exit 1
fi
printf '全部通过。临时目录已清理：%s\n' "$TMP"
exit 0
