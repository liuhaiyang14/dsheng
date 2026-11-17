//! First desktop import must work without a separate metric-registration action.
//! All files and SQLite state are synthetic and confined to a temporary directory.
use dsheng_app::contracts::{core, env::DesktopEnv};
use dsheng_cli::store::sources;
use serde_json::{json, Value};

struct Workspace {
    _temp: tempfile::TempDir,
    env: DesktopEnv,
    db: std::path::PathBuf,
    directory: String,
    id: String,
}

impl Workspace {
    fn empty() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("first-import.sqlite3");
        let env = DesktopEnv::with_db(&db);
        let source = temp.path().join("source");
        std::fs::create_dir(&source).unwrap();
        let mut book = rust_xlsxwriter::Workbook::new();
        let sheet = book.add_worksheet().set_name("付款单").unwrap();
        for (col, header) in ["data_id", "付款时间", "付款金额/元"].iter().enumerate() {
            sheet.write_string(0, col as u16, *header).unwrap();
        }
        for (row, (id, date, amount)) in [
            ("payment-1", "2026-09-12 13:45:00", "35.25"),
            ("payment-2", "2026-09-13 09:00:00", "14.75"),
        ]
        .iter()
        .enumerate()
        {
            for (col, value) in [id, date, amount].iter().enumerate() {
                sheet
                    .write_string((row + 1) as u32, col as u16, **value)
                    .unwrap();
            }
        }
        book.save(source.join("付款单.xlsx")).unwrap();
        let created = core::settings::workspace_create(
            &env,
            json!({
                "schema_version":"1", "operation":"workspace_create", "name":"首次导入回归",
                "initial_period_policy":{"cadence":"month", "effective_from":"2024-01-01",
                    "week_start_day":1, "month_start_day":1, "fiscal_year_start_month":1,
                    "timezone_id":"Asia/Shanghai"}
            }),
        );
        assert_eq!(created["ok"], true, "{created}");
        let id = created["data"]["workspace"]["workspace_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let result = Self {
            _temp: temp,
            env,
            db,
            directory: source.to_string_lossy().into_owned(),
            id,
        };
        assert!(
            result.metrics().is_empty(),
            "Workspace creation must not seed fixture metrics"
        );
        result
    }

    fn metrics(&self) -> Vec<Value> {
        sources::metric_list(
            &rusqlite::Connection::open(&self.db).unwrap(),
            self.id.parse().unwrap(),
        )
        .unwrap()
    }

    fn scan(&self) -> Value {
        let scan = core::data::scan_with_profile(
            &self.env,
            self.id.clone(),
            self.directory.clone(),
            Some("xsd_trade_v1".into()),
            false,
        );
        assert_eq!(scan["ok"], true, "{scan}");
        scan["data"].clone()
    }

    fn confirm(&self, hash: &str) {
        let result = core::data::source_mapping_set(
            &self.env,
            self.id.clone(),
            self.directory.clone(),
            hash.into(),
            json!([{
                "tool_field":"profile_mapping", "source_label":"付款单.xlsx", "source_id":"付款单.xlsx",
                "source":"付款单.xlsx", "recognized":"已确认", "usage":"已核对模板定义的文件用途和关键列",
                "target_role":"profile_mapping", "tax_treatments":{}
            }]),
        );
        assert_eq!(result["ok"], true, "{result}");
    }

    fn analyze(&self) -> Value {
        core::data::analyze(
            &self.env,
            json!({"workspace_id":self.id, "directory_label":self.directory,
            "from":"2026-09-01", "to":"2026-09-30", "grain":"month",
            "source_profile":"xsd_trade_v1", "generic":false}),
        )
    }

    fn assert_saved_month(&self, result: &Value) {
        assert_eq!(result["ok"], true, "{result}");
        let metrics = result["data"]["metrics"].as_array().unwrap();
        assert!(!metrics.is_empty());
        let payments = metrics
            .iter()
            .find(|metric| metric["metric_key"] == "payments_amount")
            .unwrap();
        assert_eq!(payments["actual_value"], "50");
        let history = core::history::history_list(&self.env, self.id.clone());
        assert_eq!(history["ok"], true, "{history}");
        let runs = history["data"]["runs"].as_array().unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0]["run_id"], result["data"]["run_id"]);
        assert_eq!(
            runs[0]["period"],
            json!({"from":"2026-09-01","to":"2026-09-30"})
        );
        assert!(
            runs[0]["summary"]["metric_count"].as_u64().unwrap() > 0,
            "Header must retain this period"
        );
        let conn = rusqlite::Connection::open(&self.db).unwrap();
        sources::require_two_tables_materialized(
            &conn,
            self.id.parse().unwrap(),
            result["data"]["run_id"].as_str().unwrap(),
            1,
        )
        .unwrap();
        let workspace = core::settings::workspace_list(&self.env);
        assert!(workspace["data"]["workspaces"][0]["active_provider_id"].is_null());
    }
}

#[test]
fn first_scan_confirm_analyze_saves_month_without_manual_metric_registration() {
    let workspace = Workspace::empty();
    let scan = workspace.scan();
    assert!(!workspace.metrics().is_empty());
    assert_eq!(
        scan["available_periods"],
        json!([{"from":"2026-09-01","to":"2026-09-30","grain":"month"}])
    );
    assert_eq!(scan["files"][0]["mapping_status"], "pending_review");
    let blocked = workspace.analyze();
    assert_eq!(
        blocked["ok"], false,
        "Automatic metric setup must retain confirmation gate"
    );
    assert_eq!(blocked["error"]["code"], "DATA_VALIDATION_FAILED");
    workspace.confirm(scan["source_profile"]["hash"].as_str().unwrap());
    assert_eq!(workspace.scan()["files"][0]["mapping_status"], "mapped");
    workspace.assert_saved_month(&workspace.analyze());
}

#[test]
fn already_confirmed_workspace_without_metrics_recovers_on_analyze() {
    let workspace = Workspace::empty();
    // This is the state left by older desktop versions after file confirmation.
    let profile = sources::resolve_source_profile(Some("xsd_trade_v1"), false).unwrap();
    workspace.confirm(&profile.hash);
    assert!(workspace.metrics().is_empty());
    workspace.assert_saved_month(&workspace.analyze());
}

#[test]
fn repeated_scan_and_analyze_keep_existing_metric_versions_unchanged() {
    let workspace = Workspace::empty();
    let scan = workspace.scan();
    let mut conn = rusqlite::Connection::open(&workspace.db).unwrap();
    // An enterprise has customized an already registered definition.
    sources::metric_set(&mut conn, workspace.id.parse().unwrap(), serde_json::from_value(json!({
        "metric_key":"payments_amount", "label":"企业付款实绩", "semantic_type":"flow",
        "unit_signature":{"dimension":"money","base_unit":"元","currency":"CNY","tax_basis":"not_applicable"},
        "direction":"neutral", "target_constraint_scope":"none", "aggregation":"sum_disjoint",
        "comparison_policy":{"positive_base":"relative_change","zero_or_negative_base":"absolute_only"},
        "effective_from":"2026-01-01", "expected_revision":1
    })).unwrap()).unwrap();
    let definitions = workspace.metrics();
    workspace.scan();
    assert_eq!(workspace.metrics(), definitions);
    workspace.confirm(scan["source_profile"]["hash"].as_str().unwrap());
    workspace.assert_saved_month(&workspace.analyze());
    assert_eq!(workspace.metrics(), definitions);
}
