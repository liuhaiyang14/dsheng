//! 合同夹具测试（Phase 0 基线 #7）：
//! 机器合同 JSON 与合成算例全部可解析、与 Rust 类型往返一致。
//! 夹具完全合成；真实业务文件不入库、不入 CI。

use dsheng_core::contracts::metrics::MetricCatalogEntry;

fn repo_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read_contract(rel: &str) -> String {
    std::fs::read_to_string(repo_root().join(rel)).expect(rel)
}

#[test]
fn metric_catalog_parses_into_frozen_types() {
    let raw = read_contract("contracts/v1/metric-catalog.json");
    let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let metrics = v["metrics"].as_array().expect("metrics 数组");
    assert_eq!(metrics.len(), 26, "目录 26 个指标");
    for m in metrics {
        let entry: MetricCatalogEntry = serde_json::from_value(m.clone())
            .unwrap_or_else(|e| panic!("指标 {} 解析失败: {e}", m["metric_key"]));
        if let Some(f) = &entry.formula {
            let known: std::collections::HashSet<&str> = metrics
                .iter()
                .map(|x| x["metric_key"].as_str().unwrap())
                .collect();
            f.validate(&known)
                .unwrap_or_else(|e| panic!("指标 {} 公式非法: {e}", entry.metric_key));
        }
    }
    // 占位替换往返：ratio/time 无占位；money 占位需确认值
    let sales = metrics
        .iter()
        .find(|m| m["metric_key"] == "sales_net")
        .map(|m| serde_json::from_value::<MetricCatalogEntry>(m.clone()).unwrap())
        .unwrap();
    assert!(sales.unit.has_placeholder());
    let materialized = sales
        .unit
        .materialize(
            Some("CNY".into()),
            Some(dsheng_core::contracts::domain::TaxBasis::Exclusive),
            None,
        )
        .expect("确认值替换后应有效");
    materialized.validate().unwrap();
}

#[test]
fn data_period_cases_load() {
    let raw = read_contract("contracts/v1/data-period-cases.json");
    let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let cases = v["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 56, "DC 算例 56 条");
    assert!(cases
        .iter()
        .all(|c| c.get("case_id").is_some_and(|x| x.is_string())));
    assert!(cases
        .iter()
        .all(|c| c.get("operation").is_some_and(|x| x.is_string())));
    assert!(cases
        .iter()
        .all(|c| c.get("input").is_some() && c.get("expected").is_some()));
}

#[test]
fn task_state_cases_load() {
    let raw = read_contract("contracts/v1/task-state-cases.json");
    let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let sv = v["schema_validation_cases"].as_array().unwrap();
    let da = v["domain_acceptance_cases"].as_array().unwrap();
    assert_eq!(sv.len(), 34, "schema 用例 34 条");
    assert_eq!(da.len(), 50, "领域用例 50 条");
    // schema 用例逐条可反序列化为 task-edit 输入
    for c in sv {
        let schema_name = c["schema"].as_str().unwrap();
        assert!(schema_name.starts_with("task-edit") || schema_name.starts_with("task-reconcile"));
        assert!(c.get("schema_valid").is_some_and(|x| x.is_boolean()));
    }
}

#[test]
fn cli_surface_catalog_matches_rust_enum() {
    let raw = read_contract("contracts/v1/cli-surface.json");
    let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let errors = v["errors"].as_object().expect("errors 对象");
    let commands = v["commands"].as_array().unwrap();
    assert_eq!(
        commands.len(),
        54,
        "54 条命令（2026-11-02 增补 analysis show）"
    );
    for (code, spec) in errors {
        let parsed: dsheng_core::contracts::cli::CliErrorCode =
            serde_json::from_value(serde_json::Value::String(code.clone()))
                .unwrap_or_else(|_| panic!("错误码 {code} 未在 Rust 枚举中"));
        let exit: i64 = spec["exit_code"].as_i64().unwrap();
        assert_eq!(
            parsed.exit_code() as i64,
            exit,
            "错误码 {code} 退出码不一致"
        );
        // INPUT_REVISION_CHANGED 派发后 4 由阶段覆盖，主映射仍为 2
    }
}

#[test]
fn task_and_diagnosis_input_schemas_are_valid_json() {
    for rel in [
        "contracts/v1/task-input.schema.json",
        "contracts/v1/diagnosis-input.schema.json",
        "contracts/v1/task-edit.schema.json",
        "contracts/v1/task-reconcile.schema.json",
        "contracts/v1/settings.schema.json",
        "crates/dsheng-core/prompts/v1/schema/task-output.schema.json",
        "crates/dsheng-core/prompts/v1/schema/diagnosis-report.schema.json",
    ] {
        let raw = read_contract(rel);
        let v: serde_json::Value =
            serde_json::from_str(&raw).unwrap_or_else(|e| panic!("{rel} 不是合法 JSON: {e}"));
        assert!(v.get("$id").is_some(), "{rel} 缺 $id");
    }
}

#[test]
fn read_views_schema_defs_and_cli_hooks() {
    let raw = read_contract("contracts/v1/read-views.schema.json");
    let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(v["$id"], "dsheng-read-views-v1");
    let defs = v["$defs"].as_object().unwrap();
    for required in [
        "scanSummaryView",
        "fieldMappingView",
        "metricRowView",
        "metricSeriesView",
        "analysisRunView",
        "taskView",
        "taskCandidateView",
        "timelineEntryView",
        "historyEntryView",
        "historyDetailView",
        "diagnosisReportSummaryView",
        "reconcilePreviewView",
        "comparisonView",
    ] {
        assert!(defs.contains_key(required), "read-views 缺 {required}");
        assert_eq!(
            defs[required]["additionalProperties"], false,
            "{required} 必须封闭字段"
        );
    }
    // cli-surface 已挂接读侧输出契约
    let cli: serde_json::Value =
        serde_json::from_str(&read_contract("contracts/v1/cli-surface.json")).unwrap();
    let hooked: Vec<&str> = cli["commands"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| {
            c["output_contract"]
                .as_str()
                .unwrap_or("")
                .contains("read-views")
        })
        .map(|c| c["command"].as_str().unwrap())
        .collect();
    for expected in [
        "scan",
        "mapping show",
        "analyze",
        "analysis show",
        "diagnosis list",
        "task list",
        "task show",
        "task timeline",
        "history list",
        "history show",
    ] {
        assert!(
            hooked.contains(&expected),
            "命令 {expected} 未挂 read-views 输出契约"
        );
    }
}
