//! 保存策略决定任务下一月周期；合成数据库，无 Provider 调用。
mod fixture;
use dsheng_cli::store::generation::build_task_payload;
use fixture::Fixture;
use rusqlite::{params, Connection};
use serde_json::{json, Value};

fn target_period(
    from: &str,
    to: &str,
    month_start_day: Option<u32>,
    explicit: Option<Value>,
) -> Value {
    let f = Fixture::new("task-month-period");
    let conn = Connection::open(&f.db).unwrap();
    let (raw, policy_raw): (String, String) = conn.query_row("SELECT normalized_metrics_json,period_policy_snapshot_json FROM run_revisions WHERE run_id=?1 AND revision_no=1",params![f.run_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    let mut normalized: Value = serde_json::from_str(&raw).unwrap();
    normalized["period"] = json!({"from":from,"to":to});
    let mut policy: Value = serde_json::from_str(&policy_raw).unwrap();
    if let Some(day) = month_start_day {
        policy["month_start_day"] = json!(day);
    } else {
        policy.as_object_mut().unwrap().remove("month_start_day");
    }
    conn.execute("UPDATE run_revisions SET normalized_metrics_json=?2,period_policy_snapshot_json=?3,action_period_snapshot_json=?4 WHERE run_id=?1 AND revision_no=1",params![f.run_id,normalized.to_string(),policy.to_string(),explicit.map(|v|v.to_string())]).unwrap();
    let material = build_task_payload(&conn, f.workspace, &f.run_id, "生成下期任务", None).unwrap();
    let payload: Value = serde_json::from_str(&material.payload_json).unwrap();
    json!({"from":payload["period"]["target_from"],"to":payload["period"]["target_to"]})
}

#[test]
fn follows_saved_month_start_instead_of_natural_month() {
    assert_eq!(
        target_period("2026-10-15", "2026-11-14", Some(15), None),
        json!({"from":"2026-11-15","to":"2026-12-14"})
    );
    assert_eq!(
        target_period("2026-12-15", "2027-01-14", Some(15), None),
        json!({"from":"2027-01-15","to":"2027-02-14"})
    );
}

#[test]
fn short_months_keep_the_saved_anchor() {
    assert_eq!(
        target_period("2026-01-31", "2026-02-27", Some(31), None),
        json!({"from":"2026-02-28","to":"2026-03-30"})
    );
    assert_eq!(
        target_period("2026-02-28", "2026-03-30", Some(31), None),
        json!({"from":"2026-03-31","to":"2026-04-29"})
    );
}

#[test]
fn old_snapshots_default_to_first_day_and_explicit_windows_are_preserved() {
    assert_eq!(
        target_period("2026-10-01", "2026-10-31", None, None),
        json!({"from":"2026-11-01","to":"2026-11-30"})
    );
    let explicit = json!({"from":"2026-12-15","to":"2027-01-14"});
    assert_eq!(
        target_period("2026-10-15", "2026-11-14", Some(15), Some(explicit.clone())),
        explicit
    );
}
