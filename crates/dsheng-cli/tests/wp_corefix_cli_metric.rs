//! 核心缺口修复 F2（CLI 侧）：stable metric identity 的写入与投影对齐。
//!
//! - metric set 升版本：新版本行新 metric_id，但 stable_metric_id 继承首版；
//! - §1.4 模型输入投影（workspace.metric_definitions[].metric_id、
//!   metrics[].metric_id、manualInputs[].metric_id）用 stable_metric_id，
//!   version/semantic_hash 仍为当前具体版本行值；
//! - 冻结引用（run_revisions 快照 / 旧 metric_id 引用行）不重写。

mod fixture;

use fixture::Fixture;
use serde_json::Value;

fn run(args: &[&str]) -> (String, i32) {
    let owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = dsheng_cli::run_with_output(&owned, &mut out, &mut err);
    (String::from_utf8(out).expect("utf8"), code)
}

fn data_of(stdout: &str) -> Value {
    let v: Value = serde_json::from_str(stdout.trim()).expect("envelope");
    assert_eq!(v["ok"], true, "{stdout}");
    v["data"].clone()
}

#[test]
fn stable_identity_survives_version_bump_and_projects_into_model_input() {
    let f = Fixture::new("f2-stable");
    let dir = &f.dir;
    // v1：fixture 建的 sales_net 首版（stable = 首版行 id）
    let stable_id = {
        let conn = rusqlite::Connection::open(&f.db).unwrap();
        let (row_id, stable): (String, String) = conn
            .query_row(
                "SELECT metric_id, stable_metric_id FROM metric_definitions
                 WHERE workspace_id = ?1 AND metric_key = 'sales_net' AND version = 1",
                rusqlite::params![f.workspace.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(row_id, f.metric_id, "首版行 id 即 CLI 返回的 metric_id");
        assert_eq!(stable, row_id, "首版 stable_metric_id = 首版行 id");
        stable
    };
    // 升版本 v2（仅改 label；expected_revision=1）
    std::fs::write(
        dir.join("metric_v2.json"),
        r#"{"metric_key":"sales_net","label":"销售额（新口径）","semantic_type":"flow","unit_signature":{"dimension":"money","base_unit":"元","currency":"CNY","tax_basis":"exclusive"},"direction":"up","target_constraint_scope":"monthly_revenue_floor","revenue_semantics":true,"aggregation":"sum_disjoint","required_inputs":[],"comparison_policy":{"positive_base":"relative_change","zero_or_negative_base":"absolute_only"},"effective_from":"2026-01-01","expected_revision":1}"#,
    )
    .unwrap();
    let (stdout, code) = run(&[
        "metric",
        "set",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace.to_string(),
        "--input-json",
        dir.join("metric_v2.json").to_string_lossy().as_ref(),
        "--json",
    ]);
    assert_eq!(code, 0, "metric set v2: {stdout}");
    let view = data_of(&stdout);
    let v2_row_id = view["metric_id"].as_str().unwrap().to_string();
    assert_ne!(v2_row_id, stable_id, "新版本行获得新 metric_id");
    assert_eq!(
        view["stable_metric_id"].as_str().unwrap(),
        stable_id,
        "升版本 stable_metric_id 不变（§1.4 保留身份）"
    );
    assert_eq!(view["version"].as_i64().unwrap(), 2);
    // DB 侧：两行共用同一 stable id
    let conn = rusqlite::Connection::open(&f.db).unwrap();
    let rows: Vec<(String, String, i64)> = {
        let mut stmt = conn
            .prepare(
                "SELECT metric_id, stable_metric_id, version FROM metric_definitions
                 WHERE workspace_id = ?1 AND metric_key = 'sales_net' ORDER BY version",
            )
            .unwrap();
        stmt.query_map(rusqlite::params![f.workspace.to_string()], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
    };
    assert_eq!(rows.len(), 2, "多版本行保留");
    for (_, stable, _) in &rows {
        assert_eq!(stable, &stable_id);
    }
    // 手工输入指向 v2 具体版本行（冻结引用形态；input_id 为真实 UUID）
    conn.execute(
        "INSERT INTO manual_inputs (input_id, workspace_id, metric_id, period_key, measure_role,
                scope_key, value, unit, confirmed, source_note, input_revision, raw_text,
                created_at, updated_at)
         VALUES (?3, ?1, ?2, '2026-10', 'actual', '全公司', '100.5', '元', 1, '备注', 1,
                 '原文', 't', 't')",
        rusqlite::params![
            f.workspace.to_string(),
            v2_row_id,
            uuid::Uuid::new_v4().to_string()
        ],
    )
    .unwrap();
    // 模型输入投影：metric_id 全部为 stable id；version/semantic_hash 为 v2 行值
    let material = dsheng_cli::store::generation::build_task_payload(
        &conn,
        f.workspace,
        &f.run_id,
        "（稳定身份核对）",
        None,
    )
    .unwrap();
    let payload: Value = serde_json::from_str(&material.payload_json).unwrap();
    let def = &payload["workspace"]["metric_definitions"][0];
    assert_eq!(def["metric_key"].as_str().unwrap(), "sales_net");
    assert_eq!(
        def["metric_id"].as_str().unwrap(),
        stable_id,
        "§1.4 定义投影 metric_id = stable_metric_id"
    );
    assert_eq!(
        def["version"].as_i64().unwrap(),
        2,
        "version 冻结当前版本行"
    );
    let metrics_arr = payload["metrics"].as_array().unwrap().clone();
    assert_eq!(metrics_arr.len(), 1, "fixture 仅 sales_net 一个指标");
    let metric_row = &metrics_arr[0];
    assert_eq!(
        metric_row["metric_id"].as_str().unwrap(),
        stable_id,
        "metrics[].metric_id = stable_metric_id"
    );
    let manual = &payload["manual_inputs"][0];
    assert_eq!(
        manual["metric_id"].as_str().unwrap(),
        stable_id,
        "manualInput.metric_id 归并为定义身份"
    );
    // 旧 run revision 快照（v1 时点的具体版本行 id）不被迁移/升版本重写
    let snapshot_raw: String = conn
        .query_row(
            "SELECT metric_definitions_snapshot_json FROM run_revisions
             WHERE run_id = ?1 AND revision_no = 1",
            rusqlite::params![f.run_id],
            |r| r.get(0),
        )
        .unwrap();
    let snapshot: Value = serde_json::from_str(&snapshot_raw).unwrap();
    assert!(
        !snapshot.to_string().contains(&v2_row_id),
        "快照仍为 v1 时点内容（冻结引用不动）"
    );
}

#[test]
fn metric_list_carries_stable_identity() {
    let f = Fixture::new("f2-list");
    let (stdout, code) = run(&[
        "metric",
        "list",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace.to_string(),
        "--json",
    ]);
    assert_eq!(code, 0, "{stdout}");
    let metrics = data_of(&stdout)["metrics"].as_array().unwrap().clone();
    assert_eq!(metrics.len(), 1);
    assert_eq!(
        metrics[0]["stable_metric_id"].as_str().unwrap(),
        f.metric_id
    );
}
