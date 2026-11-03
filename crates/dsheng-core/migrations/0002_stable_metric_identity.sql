-- dsheng SQLite schema 0002：stable metric identity（data-period-import §1.4）。
--
-- 合同：模型输入投影的 metric_id 引用工作区内稳定的定义身份，
-- metric_key 为 catalog 业务键；版本变化保留身份，analysis revision
-- 冻结具体 version 与 semantic hash。
--
-- 0001 现状：每版本一行、每版本新 metric_id（UNIQUE(workspace,metric_key,version)），
-- 「定义身份」随版本漂移。0002 在保留全部版本行（冻结引用：run_revisions
-- 快照、tasks.metric_id、manual_inputs.metric_id 等继续指向具体版本行）的
-- 前提下新增 stable_metric_id 列：
--   * 回填 = 各 (workspace_id, metric_key) 首版（MIN(version)）行的 metric_id，
--     同键全部版本行共用同一 stable_metric_id（即首版行 id 本身）；
--   * 写入路径（metric set 升版本）必须继承同一 stable_metric_id；
--   * 模型输入投影（§1.4 workspace.metric_definitions[].metric_id 与
--     metrics[].metric_id）改用 stable_metric_id。
--
-- 说明：同键多版本行保留时，SQLite 无法在 metric_definitions 上表达
-- 「每 (workspace_id, metric_key) 恰一个 stable_metric_id」的组级唯一
-- （UNIQUE(workspace_id, metric_key) 会拒绝多版本行）；该不变量由
-- 回填构造（首版 id）+ 写入路径继承 + 迁移测试共同保证。
--
-- 实现方式：SQLite 不能对已有列追加 NOT NULL 约束，按标准做法重建表
-- （0001 冻结文件不改；此处完整重声明其约束并新增列）。无外键指向
-- metric_definitions（0001 中引用列均无 REFERENCES），重建安全。

CREATE TABLE metric_definitions_0002 (
    metric_id                TEXT PRIMARY KEY,
    workspace_id             TEXT NOT NULL REFERENCES workspaces(workspace_id),
    metric_key               TEXT NOT NULL,
    version                  INTEGER NOT NULL CHECK (version >= 1),
    stable_metric_id         TEXT NOT NULL CHECK (length(stable_metric_id) > 0),
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

INSERT INTO metric_definitions_0002
    (metric_id, workspace_id, metric_key, version, stable_metric_id, semantic_hash,
     label, label_aliases_json, semantic_type, unit_signature_json, direction,
     target_constraint_scope, revenue_semantics, aggregation, formula_json,
     required_inputs_json, denominator_policy, comparison_policy_json, labor_treatment,
     effective_from, effective_to, created_at)
SELECT m.metric_id, m.workspace_id, m.metric_key, m.version,
       (SELECT f.metric_id FROM metric_definitions f
         WHERE f.workspace_id = m.workspace_id AND f.metric_key = m.metric_key
         ORDER BY f.version LIMIT 1),
       m.semantic_hash, m.label, m.label_aliases_json, m.semantic_type,
       m.unit_signature_json, m.direction, m.target_constraint_scope,
       m.revenue_semantics, m.aggregation, m.formula_json, m.required_inputs_json,
       m.denominator_policy, m.comparison_policy_json, m.labor_treatment,
       m.effective_from, m.effective_to, m.created_at
  FROM metric_definitions m;

DROP TABLE metric_definitions;
ALTER TABLE metric_definitions_0002 RENAME TO metric_definitions;

CREATE INDEX idx_metric_definitions_stable
    ON metric_definitions(workspace_id, stable_metric_id);
