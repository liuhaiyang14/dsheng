//! 分析 run 与不可变 revision（task-lifecycle.md §2.1 / data-period-import.md §3.1）。
//!
//! - period_key = 策略版本 + 粒度 + 起止 + 业务 scope；`workspace_id + period_key`
//!   定位唯一期间（0001 冻结 UNIQUE）。
//! - 相同 workspace+period_key 重做：保留 run_id，追加不可变 run_revisions 行，
//!   并把 analysis_runs.current_revision_no / 指纹指针前移；输入相同也产生新的
//!   显式重做记录，不自动把旧诊断当新版结果。
//! - 策略 [from,to) 左闭右开版本化：注册拒绝重叠；分析跨界默认
//!   `policy_selection_required`，显式选一版本作用于整段并标 override。

use crate::analysis::fingerprint::{AnalysisRevisionInput, CANONICALIZATION_VERSION};
use crate::analysis::periods as cal;
use crate::analysis::periods::PolicyVersion;
use crate::contracts::domain::{Cadence, DateRange, Granularity};
use crate::repository::{ensure_workspace, utc_iso, RepoError};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use std::collections::HashSet;

/// period_key 生成（§6.3：策略版本、粒度、本期起止日期及业务 scope）。
///
/// 分隔符 `|`；scope 以百分号转义避免歧义。同输入必同 key、必同 run。
pub fn build_period_key(
    policy_version: &str,
    grain: Granularity,
    range: DateRange,
    scope: &str,
) -> String {
    let g = serde_json::to_value(grain).expect("granularity");
    format!(
        "pv={}|g={}|from={}|to={}|scope={}",
        percent_escape(policy_version),
        g.as_str().expect("snake_case"),
        range.from.format("%Y-%m-%d"),
        range.to.format("%Y-%m-%d"),
        percent_escape(scope)
    )
}

fn percent_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '%' => out.push_str("%25"),
            '|' => out.push_str("%7C"),
            c if (c as u32) < 0x20 || c as u32 == 0x7F => {
                out.push_str(&format!("%{:02X}", c as u32))
            }
            c => out.push(c),
        }
    }
    out
}

/// 周期策略注册记录。
#[derive(Debug, Clone)]
pub struct PeriodPolicyRecord {
    pub period_policy_id: String,
    pub version: u32,
    pub cadence: Cadence,
    pub effective_from: chrono::NaiveDate,
    pub effective_to: Option<chrono::NaiveDate>,
    pub week_start_day: u32,
    pub month_start_day: u32,
    pub fiscal_year_start_month: u32,
    pub timezone_id: String,
}

/// 注册策略版本：[from,to) 与同 workspace 任何现存版本重叠即拒绝（§3.1）。
pub fn register_period_policy(
    conn: &Connection,
    workspace_id: &str,
    record: &PeriodPolicyRecord,
) -> Result<(), RepoError> {
    ensure_workspace(conn, workspace_id, "workspace")?;
    if let Some(to) = record.effective_to {
        if to <= record.effective_from {
            return Err(RepoError::PolicyOverlapRejected);
        }
    }
    let existing = load_policy_versions(conn, workspace_id)?;
    let candidate = PolicyVersion {
        version: record.version.to_string(),
        effective_from: record.effective_from,
        effective_to: record.effective_to,
    };
    // 与任何现存版本（含同名替换意图）重叠都拒绝；先移除自身旧版做纯比较
    let others: Vec<PolicyVersion> = existing
        .iter()
        .filter(|v| v.version != candidate.version)
        .cloned()
        .collect();
    let mut all = others.clone();
    all.push(candidate.clone());
    if cal::validate_policy_no_overlap(&all).is_err() {
        return Err(RepoError::PolicyOverlapRejected);
    }
    let cadence = serde_json::to_value(record.cadence)
        .expect("cadence")
        .as_str()
        .expect("snake_case")
        .to_string();
    let now = utc_iso(Utc::now());
    conn.execute(
        "INSERT INTO period_policies
           (period_policy_id, workspace_id, version, cadence, effective_from, effective_to,
            week_start_day, fiscal_year_start_month, timezone_id, created_at, month_start_day)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
         ON CONFLICT(workspace_id, version) DO UPDATE SET
            cadence=excluded.cadence, effective_from=excluded.effective_from,
            effective_to=excluded.effective_to, week_start_day=excluded.week_start_day,
            fiscal_year_start_month=excluded.fiscal_year_start_month,
            timezone_id=excluded.timezone_id, month_start_day=excluded.month_start_day",
        params![
            record.period_policy_id,
            workspace_id,
            record.version,
            cadence,
            record.effective_from.format("%Y-%m-%d").to_string(),
            record
                .effective_to
                .map(|d| d.format("%Y-%m-%d").to_string()),
            record.week_start_day,
            record.fiscal_year_start_month,
            record.timezone_id,
            now,
            record.month_start_day
        ],
    )?;
    Ok(())
}

/// 读取 workspace 的策略版本（按 effective_from 升序）。
pub fn load_policy_versions(
    conn: &Connection,
    workspace_id: &str,
) -> Result<Vec<PolicyVersion>, RepoError> {
    let mut stmt = conn.prepare(
        "SELECT version, effective_from, effective_to FROM period_policies
         WHERE workspace_id = ?1 ORDER BY effective_from",
    )?;
    let rows = stmt.query_map(params![workspace_id], |r| {
        let version: i64 = r.get(0)?;
        let from: String = r.get(1)?;
        let to: Option<String> = r.get(2)?;
        Ok((version, from, to))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (version, from, to) = row?;
        out.push(PolicyVersion {
            version: version.to_string(),
            effective_from: chrono::NaiveDate::parse_from_str(&from, "%Y-%m-%d")
                .map_err(|_| RepoError::Db(rusqlite::Error::InvalidQuery))?,
            effective_to: to
                .map(|t| {
                    chrono::NaiveDate::parse_from_str(&t, "%Y-%m-%d")
                        .map_err(|_| RepoError::Db(rusqlite::Error::InvalidQuery))
                })
                .transpose()?,
        });
    }
    Ok(out)
}

/// 为分析范围选策略（DB 版）：跨界且无显式选择 →
/// [`RepoError::PolicySelectionRequired`]。
pub fn resolve_policy_for_range(
    conn: &Connection,
    workspace_id: &str,
    range: DateRange,
    explicit_policy: Option<&str>,
) -> Result<(String, bool), RepoError> {
    let versions = load_policy_versions(conn, workspace_id)?;
    cal::resolve_policy(&versions, range, explicit_policy)
        .map(|r| (r.policy.version.clone(), r.override_applied))
        .map_err(|_| RepoError::PolicySelectionRequired)
}

/// revision 内容载荷 → run_revisions 各 JSON 列。
#[derive(Debug, Clone)]
pub struct RevisionPayload {
    pub input: AnalysisRevisionInput,
    pub action_period_snapshot: Option<DateRange>,
}

/// upsert 结果。
#[derive(Debug, Clone, PartialEq)]
pub struct RevisionUpsert {
    pub run_id: String,
    pub revision_no: u32,
    pub created_run: bool,
}

/// 创建或重做分析 revision（单事务；§2.1）。
///
/// - 首次：插入 analysis_runs（current_revision_no=1）+ run_revisions #1；
/// - 重做：保留 run_id，追加不可变 revision，更新当前指针与指纹；
/// - workspace 隔离：period_key 冲突但 workspace 不同互不影响。
pub fn upsert_analysis_revision(
    conn: &mut Connection,
    workspace_id: &str,
    period_key: &str,
    payload: &RevisionPayload,
    now: DateTime<Utc>,
) -> Result<RevisionUpsert, RepoError> {
    ensure_workspace(conn, workspace_id, "workspace")?;
    let fingerprint = payload.input.fingerprint();
    let tx = conn.transaction()?;
    let existing: Option<(String, i64)> = tx
        .query_row(
            "SELECT run_id, current_revision_no FROM analysis_runs
             WHERE workspace_id = ?1 AND period_key = ?2",
            params![workspace_id, period_key],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })?;
    let (run_id, revision_no, created_run, previous_revision) = match existing {
        Some((run_id, current)) => (run_id, (current + 1) as u32, false, Some(current as u32)),
        None => {
            let run_id = uuid::Uuid::new_v4().to_string();
            tx.execute(
                "INSERT INTO analysis_runs
                   (run_id, workspace_id, period_key, current_revision_no,
                    analysis_input_fingerprint, created_at, updated_at)
                 VALUES (?1, ?2, ?3, 1, ?4, ?5, ?5)",
                params![run_id, workspace_id, period_key, fingerprint, utc_iso(now)],
            )?;
            (run_id, 1, true, None)
        }
    };
    if revision_no > 1 {
        tx.execute(
            "UPDATE analysis_runs SET current_revision_no = ?2, analysis_input_fingerprint = ?3,
                    updated_at = ?4 WHERE run_id = ?1",
            params![run_id, revision_no, fingerprint, utc_iso(now)],
        )?;
    }
    insert_revision_row(&tx, &run_id, revision_no, &fingerprint, payload, now)?;
    // 同事务继承 period links、封存旧 revision 快照并建新 revision snapshot 0
    // （task-lifecycle §6；无任务时为显式空快照）。
    crate::repository::task_store::adopt_revision_tasks(
        &tx,
        &run_id,
        previous_revision.map(|r| r as i64),
        revision_no as i64,
        now,
    )
    .map_err(|e| RepoError::TaskService(e.to_string()))?;
    tx.commit()?;
    Ok(RevisionUpsert {
        run_id,
        revision_no,
        created_run,
    })
}

/// 追加 run_revisions 行（不可变；不 UPDATE 既有行）。
fn insert_revision_row(
    tx: &rusqlite::Transaction<'_>,
    run_id: &str,
    revision_no: u32,
    fingerprint: &str,
    payload: &RevisionPayload,
    now: DateTime<Utc>,
) -> Result<(), RepoError> {
    let input = &payload.input;
    let sources: Vec<serde_json::Value> = {
        let mut s: Vec<&crate::analysis::fingerprint::SourceSelection> =
            input.sources.iter().collect();
        s.sort_by(|a, b| {
            (&a.source_id, &a.sheet, &a.range).cmp(&(&b.source_id, &b.sheet, &b.range))
        });
        s.iter()
            .map(|x| {
                serde_json::json!({
                    "content_hash": x.content_hash,
                    "range": x.range,
                    "sheet": x.sheet,
                    "source_id": x.source_id,
                })
            })
            .collect()
    };
    let base_refs: Vec<serde_json::Value> = {
        let mut refs = input.comparison_base_refs.clone();
        refs.sort();
        refs.iter()
            .map(|r| serde_json::json!({"run_id": r.run_id, "revision_no": r.revision_no}))
            .collect()
    };
    tx.execute(
        "INSERT INTO run_revisions
           (run_id, revision_no, analysis_input_fingerprint, canonicalization_version,
            period_policy_snapshot_json, source_selection_snapshot_json, mapping_snapshot_json,
            manual_inputs_snapshot_json, metric_definitions_snapshot_json,
            normalized_metrics_json, comparisons_json, comparison_base_refs_json,
            action_period_snapshot_json, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            run_id,
            revision_no,
            fingerprint,
            CANONICALIZATION_VERSION,
            serde_json::to_string(&input.period_policy).expect("json"),
            serde_json::to_string(&sources).expect("json"),
            serde_json::to_string(&input.mappings).expect("json"),
            serde_json::to_string(&serde_json::json!(input.manual_inputs)).expect("json"),
            serde_json::to_string(&serde_json::json!(input.metric_definitions)).expect("json"),
            serde_json::to_string(&input.normalized_metrics).expect("json"),
            serde_json::to_string(&input.comparisons).expect("json"),
            serde_json::to_string(&base_refs).expect("json"),
            payload.action_period_snapshot.map(|p| {
                serde_json::to_string(&serde_json::json!({
                    "from": p.from.format("%Y-%m-%d").to_string(),
                    "to": p.to.format("%Y-%m-%d").to_string(),
                }))
                .expect("json")
            }),
            utc_iso(now),
        ],
    )?;
    Ok(())
}

/// 读取当前指针（run_id, current_revision_no, fingerprint）。
pub fn current_revision(
    conn: &Connection,
    workspace_id: &str,
    run_id: &str,
) -> Result<(u32, String), RepoError> {
    conn.query_row(
        "SELECT current_revision_no, analysis_input_fingerprint FROM analysis_runs
         WHERE run_id = ?1 AND workspace_id = ?2",
        params![run_id, workspace_id],
        |r| Ok((r.get::<_, i64>(0)? as u32, r.get(1)?)),
    )
    .map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => RepoError::WorkspaceMismatch,
        other => RepoError::Db(other),
    })
}

/// run 的全部 revision 号（升序）——不可变历史仍可读取。
pub fn revision_numbers(conn: &Connection, run_id: &str) -> Result<Vec<u32>, RepoError> {
    let mut stmt = conn
        .prepare("SELECT revision_no FROM run_revisions WHERE run_id = ?1 ORDER BY revision_no")?;
    let rows = stmt.query_map(params![run_id], |r| r.get::<_, i64>(0))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row? as u32);
    }
    Ok(out)
}

/// 引用的 run 是否属于该 workspace（跨 workspace 访问统一拒绝）。
pub fn run_belongs_to_workspace(
    conn: &Connection,
    workspace_id: &str,
    run_id: &str,
) -> Result<bool, RepoError> {
    let found: Option<String> = conn
        .query_row(
            "SELECT workspace_id FROM analysis_runs WHERE run_id = ?1",
            params![run_id],
            |r| r.get(0),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })?;
    Ok(found.as_deref() == Some(workspace_id))
}

/// 收集 run 引用集合（供删除依赖检查复用）。
pub fn runs_referenced_by_revisions(
    conn: &Connection,
    run_id: &str,
) -> Result<HashSet<String>, RepoError> {
    let mut out = HashSet::new();
    let mut stmt = conn.prepare(
        "SELECT comparison_base_refs_json FROM run_revisions
         WHERE comparison_base_refs_json LIKE ?1",
    )?;
    let rows = stmt.query_map(params![format!("%{run_id}%")], |r| {
        let json: String = r.get(0)?;
        Ok(json)
    })?;
    for row in rows {
        let json = row?;
        let v: serde_json::Value = serde_json::from_str(&json).unwrap_or(serde_json::json!([]));
        for item in v.as_array().unwrap_or(&Vec::new()) {
            // 仅统计结构化引用（run_id 出现在键值上），LIKE 只是预筛
            if item.get("run_id").and_then(|x| x.as_str()) == Some(run_id) {
                out.insert("comparison_base_ref".to_string());
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository::apply_migrations;

    fn conn() -> Connection {
        let conn = Connection::open_in_memory().expect("内存库");
        apply_migrations(&conn, "test").expect("迁移");
        conn
    }

    fn sample_input(value_change: &str) -> AnalysisRevisionInput {
        AnalysisRevisionInput {
            sources: vec![crate::analysis::fingerprint::SourceSelection {
                source_id: "s1".into(),
                sheet: Some("核算表".into()),
                range: "B4:L20".into(),
                content_hash: "abc".into(),
                source_profile_id: "generic".into(),
                source_profile_version: 1,
                source_profile_hash: "generic".into(),
            }],
            mapping_version: "m1".into(),
            mappings: serde_json::json!({}),
            metric_definitions: vec![crate::analysis::fingerprint::MetricDefSnapshot {
                metric_key: "sales_net".into(),
                version: 1,
                scope: "全公司".into(),
                semantic_hash: "h".into(),
                semantic_type: None,
                unit_signature: None,
            }],
            manual_inputs: vec![crate::analysis::fingerprint::ManualInputSnapshot {
                input_id: "mi-1".into(),
                metric_id: "m-1".into(),
                input_revision: 1,
                value: Some(value_change.into()),
            }],
            conflict_resolutions: vec![],
            period_policy: serde_json::json!({"timezone": "Asia/Shanghai"}),
            scope: "全公司".into(),
            current_period: DateRange::new(
                chrono::NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
                chrono::NaiveDate::from_ymd_opt(2026, 10, 31).unwrap(),
            )
            .unwrap(),
            target_period: None,
            normalized_metrics: serde_json::json!({}),
            comparisons: serde_json::json!({}),
            comparison_base_refs: vec![],
        }
    }

    #[test]
    fn redo_keeps_run_id_appends_revision() {
        let mut conn = conn();
        let key = build_period_key(
            "V1",
            Granularity::Month,
            sample_input("100").current_period,
            "全公司",
        );
        let first = upsert_analysis_revision(
            &mut conn,
            "ws",
            &key,
            &RevisionPayload {
                input: sample_input("100"),
                action_period_snapshot: None,
            },
            Utc::now(),
        )
        .unwrap();
        assert_eq!(first.revision_no, 1);
        assert!(first.created_run);
        // 相同输入重做：仍追加 revision 2，指针前移
        let second = upsert_analysis_revision(
            &mut conn,
            "ws",
            &key,
            &RevisionPayload {
                input: sample_input("100"),
                action_period_snapshot: None,
            },
            Utc::now(),
        )
        .unwrap();
        assert_eq!(second.run_id, first.run_id);
        assert_eq!(second.revision_no, 2);
        assert!(!second.created_run);
        assert_eq!(revision_numbers(&conn, &first.run_id).unwrap(), vec![1, 2]);
        let (current, _) = current_revision(&conn, "ws", &first.run_id).unwrap();
        assert_eq!(current, 2);
        // workspace 隔离：另一 workspace 同 period_key 是独立 run
        let other = upsert_analysis_revision(
            &mut conn,
            "ws2",
            &key,
            &RevisionPayload {
                input: sample_input("100"),
                action_period_snapshot: None,
            },
            Utc::now(),
        )
        .unwrap();
        assert_ne!(other.run_id, first.run_id);
    }

    #[test]
    fn manual_input_change_changes_fingerprint() {
        let a = sample_input("100");
        let b = sample_input("105");
        assert_ne!(a.fingerprint(), b.fingerprint());
    }

    #[test]
    fn policy_registration_rejects_overlap() {
        let conn = conn();
        let base = PeriodPolicyRecord {
            period_policy_id: "p1".into(),
            version: 1,
            cadence: Cadence::Month,
            effective_from: chrono::NaiveDate::from_ymd_opt(1900, 1, 1).unwrap(),
            effective_to: Some(chrono::NaiveDate::from_ymd_opt(2026, 10, 16).unwrap()),
            week_start_day: 1,
            month_start_day: 1,
            fiscal_year_start_month: 1,
            timezone_id: "Asia/Shanghai".into(),
        };
        register_period_policy(&conn, "ws", &base).unwrap();
        let overlapping = PeriodPolicyRecord {
            period_policy_id: "p2".into(),
            version: 2,
            cadence: Cadence::Month,
            effective_from: chrono::NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
            effective_to: None,
            ..base.clone()
        };
        assert!(matches!(
            register_period_policy(&conn, "ws", &overlapping),
            Err(RepoError::PolicyOverlapRejected)
        ));
        // [from,to) 左闭右开：恰接前版 effective_to 不算重叠
        let adjacent = PeriodPolicyRecord {
            period_policy_id: "p3".into(),
            version: 2,
            effective_from: chrono::NaiveDate::from_ymd_opt(2026, 10, 16).unwrap(),
            effective_to: None,
            ..base
        };
        register_period_policy(&conn, "ws", &adjacent).unwrap();
    }

    #[test]
    fn cross_boundary_requires_explicit_policy() {
        let conn = conn();
        register_period_policy(
            &conn,
            "ws",
            &PeriodPolicyRecord {
                period_policy_id: "p1".into(),
                version: 1,
                cadence: Cadence::Month,
                effective_from: chrono::NaiveDate::from_ymd_opt(1900, 1, 1).unwrap(),
                effective_to: Some(chrono::NaiveDate::from_ymd_opt(2026, 10, 16).unwrap()),
                week_start_day: 1,
                month_start_day: 1,
                fiscal_year_start_month: 1,
                timezone_id: "Asia/Shanghai".into(),
            },
        )
        .unwrap();
        register_period_policy(
            &conn,
            "ws",
            &PeriodPolicyRecord {
                period_policy_id: "p2".into(),
                version: 2,
                cadence: Cadence::Month,
                effective_from: chrono::NaiveDate::from_ymd_opt(2026, 10, 16).unwrap(),
                effective_to: None,
                week_start_day: 1,
                month_start_day: 1,
                fiscal_year_start_month: 1,
                timezone_id: "Asia/Shanghai".into(),
            },
        )
        .unwrap();
        let range = DateRange::new(
            chrono::NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
            chrono::NaiveDate::from_ymd_opt(2026, 10, 31).unwrap(),
        )
        .unwrap();
        assert!(matches!(
            resolve_policy_for_range(&conn, "ws", range, None),
            Err(RepoError::PolicySelectionRequired)
        ));
        let (applied, over) = resolve_policy_for_range(&conn, "ws", range, Some("1")).unwrap();
        assert_eq!(applied, "1");
        assert!(over);
    }

    #[test]
    fn period_key_deterministic_and_scoped() {
        let r = DateRange::new(
            chrono::NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
            chrono::NaiveDate::from_ymd_opt(2026, 10, 31).unwrap(),
        )
        .unwrap();
        let a = build_period_key("V1", Granularity::Month, r, "全公司");
        let b = build_period_key("V1", Granularity::Month, r, "全公司");
        assert_eq!(a, b);
        assert_ne!(a, build_period_key("V2", Granularity::Month, r, "全公司"));
        assert_ne!(a, build_period_key("V1", Granularity::Month, r, "华东|区"));
        let escaped = build_period_key("V1", Granularity::Month, r, "含|分隔");
        assert!(escaped.contains("%7C"), "scope 中的 | 必须转义: {escaped}");
        // 4 个字段分隔符 + scope 内已转义，共 5 段
        assert_eq!(escaped.split('|').count(), 5, "转义后不允许裸 |: {escaped}");
    }
}
