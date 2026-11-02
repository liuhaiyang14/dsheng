-- dsheng SQLite schema 0001（Phase 0 冻结基线，主 Agent 所有权）。
-- 合同来源：docs/specs/task-lifecycle.md §7–§9、provider-config-delivery.md §1/§4/§5、
-- data-period-import.md §1–§2、settings.schema.json。
-- 规则：
--  * 所有跨 workspace 访问在服务层校验 WORKSPACE_MISMATCH；
--  * 整数 revision 正值；不可变记录不 UPDATE（删除流程除外）；
--  * 时间戳为 UTC ISO-8601 字符串；
--  * api_key 按产品决策明文保存，永不进入任何公开视图/日志/导出。

PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;

CREATE TABLE app_schema_metadata (
    schema_version          INTEGER NOT NULL,
    migration_state         TEXT NOT NULL,
    last_migrated_by_app_version TEXT,
    applied_at              TEXT NOT NULL
);

-- ---- 工作区 / 周期策略 / Provider ----

CREATE TABLE workspaces (
    workspace_id             TEXT PRIMARY KEY,
    name                     TEXT NOT NULL,
    config_revision         INTEGER NOT NULL DEFAULT 1 CHECK (config_revision >= 1),
    active_provider_id      TEXT,
    current_period_policy_id TEXT,
    created_at              TEXT NOT NULL,
    updated_at              TEXT NOT NULL
);

CREATE TABLE app_preferences (
    id                       INTEGER PRIMARY KEY CHECK (id = 1),
    active_workspace_id     TEXT REFERENCES workspaces(workspace_id)
);

CREATE TABLE period_policies (
    period_policy_id         TEXT PRIMARY KEY,
    workspace_id             TEXT NOT NULL REFERENCES workspaces(workspace_id),
    version                  INTEGER NOT NULL CHECK (version >= 1),
    cadence                  TEXT NOT NULL CHECK (cadence IN ('day','week','month','quarter')),
    effective_from           TEXT NOT NULL,
    effective_to             TEXT,
    week_start_day           INTEGER NOT NULL DEFAULT 1 CHECK (week_start_day BETWEEN 1 AND 7),
    fiscal_year_start_month  INTEGER NOT NULL DEFAULT 1 CHECK (fiscal_year_start_month BETWEEN 1 AND 12),
    timezone_id              TEXT NOT NULL DEFAULT 'Asia/Shanghai',
    created_at               TEXT NOT NULL,
    UNIQUE (workspace_id, version),
    CHECK (effective_to IS NULL OR effective_to > effective_from)
);
CREATE INDEX idx_period_policies_ws_effective ON period_policies(workspace_id, effective_from);

CREATE TABLE providers (
    provider_id              TEXT PRIMARY KEY,
    workspace_id             TEXT NOT NULL REFERENCES workspaces(workspace_id),
    revision                 INTEGER NOT NULL DEFAULT 1 CHECK (revision >= 1),
    name                     TEXT NOT NULL,
    protocol                 TEXT NOT NULL DEFAULT 'chat_completions' CHECK (protocol = 'chat_completions'),
    base_url                 TEXT NOT NULL,
    model                    TEXT NOT NULL,
    stream                   INTEGER NOT NULL DEFAULT 0 CHECK (stream = 0),
    -- 明文 Key：已确认产品选择；仅服务层内存使用，任何查询不得投影本列
    api_key                  TEXT,
    connect_seconds          INTEGER NOT NULL DEFAULT 15 CHECK (connect_seconds BETWEEN 1 AND 60),
    response_seconds         INTEGER NOT NULL DEFAULT 180 CHECK (response_seconds BETWEEN 30 AND 600),
    total_seconds            INTEGER NOT NULL DEFAULT 240 CHECK (total_seconds BETWEEN 60 AND 900),
    context_window_tokens    INTEGER NOT NULL DEFAULT 32768,
    output_reserve_tokens    INTEGER NOT NULL DEFAULT 8192,
    created_at               TEXT NOT NULL,
    updated_at               TEXT NOT NULL,
    UNIQUE (workspace_id, name),
    CHECK (total_seconds >= connect_seconds + response_seconds),
    CHECK (output_reserve_tokens * 2 <= context_window_tokens)
);

-- ---- 指标定义（工作区 UUID 身份、版本、semantic hash） ----

CREATE TABLE metric_definitions (
    metric_id                TEXT PRIMARY KEY,
    workspace_id             TEXT NOT NULL REFERENCES workspaces(workspace_id),
    metric_key               TEXT NOT NULL,
    version                  INTEGER NOT NULL CHECK (version >= 1),
    semantic_hash            TEXT NOT NULL,
    label                    TEXT NOT NULL,
    label_aliases_json       TEXT NOT NULL DEFAULT '[]',
    semantic_type            TEXT NOT NULL CHECK (semantic_type IN ('flow','stock','ratio','configured_derived')),
    unit_signature_json      TEXT NOT NULL,
    direction                TEXT NOT NULL CHECK (direction IN ('up','down','neutral')),
    target_constraint_scope  TEXT NOT NULL CHECK (target_constraint_scope IN ('monthly_revenue_floor','explicit_target_only','none')),
    revenue_semantics        INTEGER NOT NULL DEFAULT 0,
    aggregation              TEXT NOT NULL,
    formula_json             TEXT,
    required_inputs_json     TEXT NOT NULL DEFAULT '[]',
    denominator_policy       TEXT NOT NULL DEFAULT 'positive_only',
    comparison_policy_json   TEXT NOT NULL,
    labor_treatment          TEXT CHECK (labor_treatment IN ('included_in_expense','separate_from_expense') OR labor_treatment IS NULL),
    effective_from           TEXT NOT NULL,
    effective_to             TEXT,
    created_at               TEXT NOT NULL,
    UNIQUE (workspace_id, metric_key, version)
);

-- ---- 来源文件 / 冲突 / 手工输入 ----

CREATE TABLE source_files (
    source_id                TEXT PRIMARY KEY,
    workspace_id             TEXT NOT NULL REFERENCES workspaces(workspace_id),
    file_label               TEXT NOT NULL,
    path_ref                 TEXT NOT NULL,
    kind                     TEXT NOT NULL CHECK (kind IN ('xlsx','xls','csv','unsupported')),
    content_hash             TEXT NOT NULL,
    created_at               TEXT NOT NULL,
    updated_at               TEXT NOT NULL
);

CREATE TABLE source_conflicts (
    conflict_id              TEXT PRIMARY KEY,
    workspace_id             TEXT NOT NULL REFERENCES workspaces(workspace_id),
    run_id                   TEXT NOT NULL,
    metric_id                TEXT NOT NULL,
    measure_role             TEXT NOT NULL CHECK (measure_role IN ('actual','plan','annual_month_plan')),
    period_from              TEXT NOT NULL,
    period_to                TEXT NOT NULL,
    scope_key                TEXT NOT NULL,
    unit                     TEXT NOT NULL,
    candidate_values_json    TEXT NOT NULL,
    resolution_status        TEXT NOT NULL DEFAULT 'unresolved' CHECK (resolution_status IN ('unresolved','resolved')),
    resolution_id            TEXT,
    selected_evidence_id     TEXT,
    resolution_note          TEXT,
    created_at               TEXT NOT NULL,
    resolved_at              TEXT,
    UNIQUE (run_id, metric_id, measure_role, period_from, period_to, scope_key, unit),
    CHECK ((resolution_status = 'resolved'
                AND resolution_id IS NOT NULL
                AND selected_evidence_id IS NOT NULL
                AND resolution_note IS NOT NULL)
        OR (resolution_status = 'unresolved'
                AND resolution_id IS NULL
                AND selected_evidence_id IS NULL
                AND resolution_note IS NULL))
);

CREATE TABLE manual_inputs (
    input_id                 TEXT PRIMARY KEY,
    workspace_id             TEXT NOT NULL REFERENCES workspaces(workspace_id),
    metric_id                TEXT NOT NULL,
    period_key               TEXT NOT NULL,
    measure_role             TEXT NOT NULL CHECK (measure_role IN ('actual','plan','annual_month_plan')),
    scope_key                TEXT NOT NULL,
    value                    TEXT,
    unit                     TEXT,
    confirmed                INTEGER NOT NULL DEFAULT 0,
    source_note              TEXT NOT NULL,
    input_revision           INTEGER NOT NULL CHECK (input_revision >= 1),
    raw_text                 TEXT NOT NULL,
    created_at               TEXT NOT NULL,
    updated_at               TEXT NOT NULL,
    UNIQUE (workspace_id, period_key, metric_id)
);

-- ---- 分析 run / 不可变 revision ----

CREATE TABLE analysis_runs (
    run_id                   TEXT PRIMARY KEY,
    workspace_id             TEXT NOT NULL REFERENCES workspaces(workspace_id),
    period_key               TEXT NOT NULL,
    current_revision_no      INTEGER NOT NULL DEFAULT 1 CHECK (current_revision_no >= 1),
    analysis_input_fingerprint TEXT NOT NULL,
    created_at               TEXT NOT NULL,
    updated_at               TEXT NOT NULL,
    UNIQUE (workspace_id, period_key)
);

CREATE TABLE run_revisions (
    run_id                   TEXT NOT NULL REFERENCES analysis_runs(run_id),
    revision_no              INTEGER NOT NULL CHECK (revision_no >= 1),
    analysis_input_fingerprint TEXT NOT NULL,
    canonicalization_version TEXT NOT NULL DEFAULT 'dsheng-canonical-json-v1',
    period_policy_snapshot_json TEXT NOT NULL,
    source_selection_snapshot_json TEXT NOT NULL,
    mapping_snapshot_json    TEXT NOT NULL,
    manual_inputs_snapshot_json TEXT NOT NULL,
    metric_definitions_snapshot_json TEXT NOT NULL,
    normalized_metrics_json  TEXT NOT NULL,
    comparisons_json         TEXT,
    comparison_base_refs_json TEXT NOT NULL DEFAULT '[]',
    action_period_snapshot_json TEXT,
    created_at               TEXT NOT NULL,
    PRIMARY KEY (run_id, revision_no)
);

-- ---- 生成会话 / 确认 / attempt ----

CREATE TABLE confirmations (
    confirmation_id          TEXT PRIMARY KEY,
    workspace_id             TEXT NOT NULL REFERENCES workspaces(workspace_id),
    workflow                 TEXT NOT NULL CHECK (workflow IN ('task','diagnosis')),
    operation                TEXT NOT NULL CHECK (operation IN ('task_generate','diagnosis_generate','diagnosis_continue','generation_retry')),
    provider_id              TEXT NOT NULL,
    provider_revision        INTEGER NOT NULL,
    run_id                   TEXT,
    analysis_revision_no     INTEGER,
    analysis_input_fingerprint TEXT NOT NULL,
    generation_input_fingerprint TEXT NOT NULL,
    report_id                TEXT,
    report_revision_no       INTEGER,
    prompt_version           TEXT NOT NULL,
    schema_version           TEXT NOT NULL,
    context_policy_version   TEXT,
    source_manifest_hash     TEXT NOT NULL,
    input_hash               TEXT NOT NULL,
    request_bytes            INTEGER NOT NULL,
    conservative_input_tokens INTEGER NOT NULL,
    budget_json              TEXT NOT NULL,
    timeouts_json            TEXT NOT NULL,
    binding_hash             TEXT NOT NULL,
    created_at               TEXT NOT NULL,
    expires_at               TEXT NOT NULL,
    consumed_at              TEXT,
    consumed_by_attempt_id   TEXT
);

CREATE TABLE generation_sessions (
    session_id               TEXT PRIMARY KEY,
    workspace_id             TEXT NOT NULL REFERENCES workspaces(workspace_id),
    run_id                   TEXT NOT NULL,
    analysis_revision_no     INTEGER NOT NULL CHECK (analysis_revision_no >= 1),
    analysis_input_fingerprint TEXT NOT NULL,
    generation_input_fingerprint TEXT NOT NULL,
    workflow                 TEXT NOT NULL CHECK (workflow IN ('task','diagnosis')),
    framework                TEXT CHECK (framework IN ('5why','5w','5w2h') OR framework IS NULL),
    state                    TEXT NOT NULL,
    prompt_version           TEXT NOT NULL,
    schema_version           TEXT NOT NULL,
    input_schema_version     TEXT NOT NULL,
    context_policy_version   TEXT,
    source_manifest_hash     TEXT NOT NULL,
    scope_snapshot_json      TEXT,
    created_at               TEXT NOT NULL,
    updated_at               TEXT NOT NULL,
    frozen_at                TEXT,
    frozen_reason            TEXT,
    CHECK ((workflow = 'task' AND framework IS NULL) OR (workflow = 'diagnosis' AND framework IS NOT NULL))
);

CREATE TABLE generation_attempts (
    attempt_id               TEXT PRIMARY KEY,
    workspace_id             TEXT NOT NULL,
    session_id               TEXT NOT NULL REFERENCES generation_sessions(session_id),
    workflow                 TEXT NOT NULL CHECK (workflow IN ('task','diagnosis')),
    operation                TEXT NOT NULL CHECK (operation IN ('task_generate','diagnosis_generate','diagnosis_continue','generation_retry')),
    confirmation_id          TEXT NOT NULL,
    provider_id              TEXT NOT NULL,
    provider_revision        INTEGER NOT NULL,
    -- Provider 快照：name/base_url/model 等；永不包含 Key
    provider_snapshot_json   TEXT NOT NULL,
    request_stage            TEXT NOT NULL DEFAULT 'preparing',
    request_count            INTEGER NOT NULL DEFAULT 0 CHECK (request_count IN (0,1)),
    prompt_version           TEXT NOT NULL,
    schema_version           TEXT NOT NULL,
    input_hash               TEXT NOT NULL,
    request_bytes            INTEGER,
    conservative_input_tokens INTEGER,
    attempt_seq              INTEGER NOT NULL CHECK (attempt_seq >= 1),
    retry_of_attempt_id      TEXT,
    dispatched_at            TEXT,
    finished_at              TEXT,
    terminal_state           TEXT CHECK (terminal_state IN ('success','failed','cancelled','interrupted','timed_out')),
    error_code               TEXT,
    error_details_json       TEXT,
    created_at               TEXT NOT NULL,
    UNIQUE (session_id, attempt_seq)
);
CREATE INDEX idx_attempts_confirmation ON generation_attempts(confirmation_id);

-- ---- 诊断报告 / 轮次 ----

CREATE TABLE diagnosis_reports (
    report_id                TEXT PRIMARY KEY,
    workspace_id             TEXT NOT NULL REFERENCES workspaces(workspace_id),
    run_id                   TEXT NOT NULL,
    analysis_revision_no     INTEGER NOT NULL CHECK (analysis_revision_no >= 1),
    analysis_input_fingerprint TEXT NOT NULL,
    report_revision_no       INTEGER NOT NULL CHECK (report_revision_no >= 1),
    session_id               TEXT NOT NULL REFERENCES generation_sessions(session_id),
    framework                TEXT NOT NULL CHECK (framework IN ('5why','5w','5w2h')),
    problem_statement        TEXT NOT NULL,
    focus_metric_id          TEXT,
    -- 完整 schema3 报告（不可变 revision snapshot）
    report_json              TEXT NOT NULL,
    state                    TEXT NOT NULL,
    created_at               TEXT NOT NULL,
    updated_at               TEXT NOT NULL,
    UNIQUE (report_id, report_revision_no)
);

CREATE TABLE diagnosis_turns (
    turn_id                  TEXT PRIMARY KEY,
    session_id               TEXT NOT NULL REFERENCES generation_sessions(session_id),
    workspace_id             TEXT NOT NULL,
    turn_no                  INTEGER NOT NULL CHECK (turn_no >= 1),
    round_no                 INTEGER NOT NULL CHECK (round_no >= 1),
    -- 用户原文与用户可见回复；未提交模型正文不落库
    user_message             TEXT NOT NULL,
    model_reply_visible      TEXT,
    turn_kind                TEXT NOT NULL,
    turn_state               TEXT NOT NULL,
    accepted_facts_json      TEXT NOT NULL DEFAULT '[]',
    report_patch_json        TEXT,
    report_revision_after    INTEGER,
    context_manifest_json    TEXT NOT NULL,
    prompt_version           TEXT NOT NULL,
    schema_version           TEXT NOT NULL,
    context_policy_version   TEXT,
    provider_id              TEXT,
    provider_revision        INTEGER,
    attempt_id               TEXT,
    clarification_questions_json TEXT,
    redirect_message         TEXT,
    stop_reason              TEXT,
    created_at               TEXT NOT NULL,
    UNIQUE (session_id, turn_no)
);

-- ---- 任务候选 / 任务 / 步骤 / 事件 / 快照 ----

CREATE TABLE task_candidates (
    candidate_generation_id  TEXT PRIMARY KEY,
    workspace_id             TEXT NOT NULL REFERENCES workspaces(workspace_id),
    run_id                   TEXT NOT NULL,
    analysis_revision_no     INTEGER NOT NULL,
    analysis_input_fingerprint TEXT NOT NULL,
    generation_input_fingerprint TEXT NOT NULL,
    candidate_revision_no    INTEGER NOT NULL DEFAULT 1 CHECK (candidate_revision_no >= 1),
    attempt_id               TEXT NOT NULL,
    -- 原始候选 JSON（模型输出原样；decimal 词法保留）
    output_json              TEXT NOT NULL,
    envelope_locally_reconstructed INTEGER NOT NULL DEFAULT 0 CHECK (envelope_locally_reconstructed IN (0,1)),
    state                    TEXT NOT NULL DEFAULT 'candidate' CHECK (state IN ('candidate','applied','discarded')),
    applied_at               TEXT,
    prompt_version           TEXT NOT NULL,
    schema_version           TEXT NOT NULL,
    created_at               TEXT NOT NULL
);

CREATE TABLE tasks (
    task_id                  TEXT PRIMARY KEY,
    workspace_id             TEXT NOT NULL REFERENCES workspaces(workspace_id),
    origin_run_id            TEXT,
    origin_revision_no       INTEGER,
    task_revision_no         INTEGER NOT NULL DEFAULT 1 CHECK (task_revision_no >= 1),
    title                    TEXT NOT NULL,
    summary                  TEXT NOT NULL DEFAULT '',
    category                 TEXT NOT NULL CHECK (category IN ('revenue','cost','efficiency','other')),
    metric_id                TEXT,
    baseline_json            TEXT,
    target_value             TEXT,
    planned_value            TEXT,
    actual_value             TEXT,
    current_value            TEXT,
    unit                     TEXT,
    mp_label                 TEXT,
    mp_unit                  TEXT,
    owner                    TEXT,
    planned_start            TEXT,
    due_date                 TEXT,
    target_period_json       TEXT,
    resource_need            TEXT,
    success_criteria         TEXT,
    assumptions_json         TEXT NOT NULL DEFAULT '[]',
    needs_review_json        TEXT NOT NULL DEFAULT '[]',
    smart_check_json         TEXT,
    smart_status             TEXT NOT NULL DEFAULT 'generated' CHECK (smart_status IN ('generated','stale')),
    assessed_task_revision   INTEGER,
    status                   TEXT NOT NULL DEFAULT 'not_started' CHECK (status IN ('not_started','in_progress','completed','blocked')),
    lifecycle                TEXT NOT NULL DEFAULT 'active' CHECK (lifecycle IN ('active','superseded')),
    superseded_by_task_id    TEXT,
    progress                 TEXT NOT NULL DEFAULT '0',
    progress_mode            TEXT NOT NULL DEFAULT 'auto' CHECK (progress_mode IN ('auto','manual')),
    adjustment_reason        TEXT,
    blocked_reason           TEXT,
    review_note              TEXT,
    completed_at             TEXT,
    created_at               TEXT NOT NULL,
    updated_at               TEXT NOT NULL
);
CREATE INDEX idx_tasks_workspace ON tasks(workspace_id, updated_at);

CREATE TABLE task_steps (
    step_id                  TEXT PRIMARY KEY,
    task_id                  TEXT NOT NULL REFERENCES tasks(task_id),
    step_order               INTEGER NOT NULL CHECK (step_order >= 1),
    action                   TEXT NOT NULL,
    owner                    TEXT,
    start_date               TEXT,
    due_date                 TEXT,
    done_criteria            TEXT,
    status                   TEXT NOT NULL DEFAULT 'not_started' CHECK (status IN ('not_started','in_progress','completed','blocked')),
    blocked_reason           TEXT,
    completed_at             TEXT,
    deleted_at               TEXT,
    created_at               TEXT NOT NULL,
    updated_at               TEXT NOT NULL
);
CREATE UNIQUE INDEX idx_task_steps_live_order ON task_steps(task_id, step_order) WHERE deleted_at IS NULL;
CREATE INDEX idx_task_steps_task ON task_steps(task_id);

CREATE TABLE task_revisions (
    task_id                  TEXT NOT NULL REFERENCES tasks(task_id),
    revision_no              INTEGER NOT NULL CHECK (revision_no >= 1),
    -- 完整规范化 task/steps/evidence 快照
    fields_snapshot_json     TEXT NOT NULL,
    actor_source             TEXT NOT NULL CHECK (actor_source IN ('ui','cli','import','generation')),
    mutation_id              TEXT NOT NULL,
    created_at               TEXT NOT NULL,
    PRIMARY KEY (task_id, revision_no)
);

CREATE TABLE task_period_links (
    id                       TEXT PRIMARY KEY,
    workspace_id             TEXT NOT NULL,
    task_id                  TEXT NOT NULL REFERENCES tasks(task_id),
    run_id                   TEXT NOT NULL,
    analysis_revision_no     INTEGER NOT NULL CHECK (analysis_revision_no >= 1),
    cycle_role               TEXT NOT NULL CHECK (cycle_role IN ('review','plan')),
    source_period_from       TEXT NOT NULL,
    source_period_to         TEXT NOT NULL,
    target_period_from       TEXT NOT NULL,
    target_period_to         TEXT NOT NULL,
    inherited_from_link_id   TEXT,
    created_at               TEXT NOT NULL,
    UNIQUE (task_id, run_id, analysis_revision_no, cycle_role)
);

CREATE TABLE task_events (
    event_id                 TEXT PRIMARY KEY,
    task_id                  TEXT NOT NULL REFERENCES tasks(task_id),
    event_seq                INTEGER NOT NULL,
    task_revision_no         INTEGER NOT NULL,
    event_type               TEXT NOT NULL,
    actor_source             TEXT NOT NULL CHECK (actor_source IN ('ui','cli','import','generation')),
    mutation_id              TEXT NOT NULL,
    before_json              TEXT,
    after_json               TEXT,
    reason                   TEXT,
    occurred_at              TEXT NOT NULL,
    UNIQUE (event_seq)
);
CREATE INDEX idx_task_events_task ON task_events(task_id, occurred_at);

CREATE TABLE task_evidence (
    id                       INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id                  TEXT NOT NULL,
    source_id                TEXT NOT NULL,
    analysis_revision_no     INTEGER NOT NULL,
    evidence_id              TEXT NOT NULL,
    summary_json             TEXT NOT NULL,
    locator                  TEXT,
    created_at               TEXT NOT NULL,
    UNIQUE (task_id, evidence_id, analysis_revision_no)
);

CREATE TABLE task_step_evidence (
    id                       INTEGER PRIMARY KEY AUTOINCREMENT,
    step_id                  TEXT NOT NULL,
    source_id                TEXT NOT NULL,
    analysis_revision_no     INTEGER NOT NULL,
    evidence_id              TEXT NOT NULL,
    summary_json             TEXT NOT NULL,
    created_at               TEXT NOT NULL,
    UNIQUE (step_id, evidence_id, analysis_revision_no)
);

CREATE TABLE run_task_snapshots (
    run_id                   TEXT NOT NULL,
    analysis_revision_no     INTEGER NOT NULL CHECK (analysis_revision_no >= 1),
    task_snapshot_no         INTEGER NOT NULL CHECK (task_snapshot_no >= 0),
    task_snapshot_json       TEXT NOT NULL,
    event_cutoff_seq         INTEGER NOT NULL,
    captured_at              TEXT NOT NULL,
    sealed                   INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (run_id, analysis_revision_no, task_snapshot_no)
);

CREATE TABLE task_candidate_applications (
    candidate_generation_id  TEXT PRIMARY KEY REFERENCES task_candidates(candidate_generation_id),
    workspace_id             TEXT NOT NULL,
    mutation_id              TEXT NOT NULL,
    mappings_json            TEXT NOT NULL,
    permanent_ids_json       TEXT NOT NULL,
    applied_result_json      TEXT NOT NULL,
    actor_source             TEXT NOT NULL CHECK (actor_source IN ('ui','cli','import','generation')),
    applied_at               TEXT NOT NULL
);

-- ---- 幂等 / 本地操作凭据 ----

CREATE TABLE local_mutations (
    workspace_id             TEXT NOT NULL,
    mutation_id              TEXT NOT NULL,
    request_digest           TEXT NOT NULL,
    result_json              TEXT,
    result_deleted           INTEGER NOT NULL DEFAULT 0 CHECK (result_deleted IN (0,1)),
    affected_run_ids_json    TEXT NOT NULL DEFAULT '[]',
    affected_task_ids_json   TEXT NOT NULL DEFAULT '[]',
    committed_at             TEXT NOT NULL,
    PRIMARY KEY (workspace_id, mutation_id)
);

-- 本地短期确认凭据（reconcile apply token / 删除 preview）：
-- 只保存绑定 digest，不记录全文，不外发。
CREATE TABLE local_action_tokens (
    token_id                 TEXT PRIMARY KEY,
    kind                     TEXT NOT NULL CHECK (kind IN ('reconcile_apply','history_delete')),
    workspace_id             TEXT NOT NULL,
    binding_digest           TEXT NOT NULL,
    created_at               TEXT NOT NULL,
    expires_at               TEXT NOT NULL,
    consumed_at              TEXT
);
