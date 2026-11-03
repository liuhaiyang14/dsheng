//! S3 门禁测试共用夹具（测试层次：临时 SQLite＋合成 XLSX；无网络）。
//!
//! 建：workspace（month 策略）＋sales_net 指标＋带 Key provider（测试注入行；
//! 不经 --api-key-stdin 真实 stdin）＋一次 analyze（产生 run）。

use rust_xlsxwriter::Workbook;
use uuid::Uuid;

#[allow(dead_code)]
pub struct Fixture {
    pub db: String,
    pub dir: std::path::PathBuf,
    pub workspace: Uuid,
    pub provider: Uuid,
    pub run_id: String,
    pub metric_id: String,
}

impl Fixture {
    pub fn new(tag: &str) -> Self {
        let tmp = tempfile::tempdir().expect("临时目录");
        let dir = tmp.keep();
        let db = dir
            .join(format!("{tag}.sqlite3"))
            .to_string_lossy()
            .to_string();
        // workspace + 指标（复用 S2 已验证的 CLI 路径）
        let run = |args: &[&str]| -> (String, i32) {
            let owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
            let mut out = Vec::new();
            let mut err = Vec::new();
            let code = dsheng_cli::run_with_output(&owned, &mut out, &mut err);
            (String::from_utf8(out).expect("utf8"), code)
        };
        std::fs::write(
            dir.join("ws.json"),
            r#"{"schema_version":"1","operation":"workspace_create","name":"合成门店","initial_period_policy":{"cadence":"month","effective_from":"2026-01-01"}}"#,
        )
        .unwrap();
        let (out, code) = run(&[
            "workspace",
            "create",
            "--db",
            &db,
            "--input-json",
            &dir.join("ws.json").to_string_lossy(),
            "--json",
        ]);
        assert_eq!(code, 0, "{out}");
        let workspace: Uuid = serde_json::from_str::<serde_json::Value>(&out).unwrap()["data"]
            ["workspace"]["workspace_id"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        std::fs::write(
            dir.join("metric.json"),
            r#"{"metric_key":"sales_net","label":"销售额","semantic_type":"flow","unit_signature":{"dimension":"money","base_unit":"元","currency":"CNY","tax_basis":"exclusive"},"direction":"up","target_constraint_scope":"monthly_revenue_floor","revenue_semantics":true,"aggregation":"sum_disjoint","required_inputs":[],"comparison_policy":{"positive_base":"relative_change","zero_or_negative_base":"absolute_only"},"effective_from":"2026-01-01","expected_revision":0}"#,
        )
        .unwrap();
        let (out, code) = run(&[
            "metric",
            "set",
            "--db",
            &db,
            "--workspace",
            &workspace.to_string(),
            "--input-json",
            &dir.join("metric.json").to_string_lossy(),
            "--json",
        ]);
        assert_eq!(code, 0, "{out}");
        let metric_id = serde_json::from_str::<serde_json::Value>(&out).unwrap()["data"]
            ["metric_id"]
            .as_str()
            .unwrap()
            .to_string();
        // provider（带测试 Key；密文不入任何输出）
        let provider = Uuid::new_v4();
        {
            let conn = rusqlite::Connection::open(&db).unwrap();
            dsheng_core::repository::apply_migrations(&conn, "test").unwrap();
            conn.execute(
                "INSERT INTO providers (provider_id, workspace_id, revision, name, protocol,
                        base_url, model, stream, api_key, connect_seconds, response_seconds,
                        total_seconds, context_window_tokens, output_reserve_tokens,
                        created_at, updated_at)
                 VALUES (?1, ?2, 1, '合成Provider', 'chat_completions',
                         'https://api.example.com/v1', 'synthetic-model', 0, 'test-key-not-real',
                         15, 180, 240, 65536, 8192, '2026-11-03T00:00:00Z', '2026-11-03T00:00:00Z')",
                rusqlite::params![provider.to_string(), workspace.to_string()],
            )
            .unwrap();
            conn.execute(
                "UPDATE workspaces SET active_provider_id = ?2 WHERE workspace_id = ?1",
                rusqlite::params![workspace.to_string(), provider.to_string()],
            )
            .unwrap();
        }
        // 合成核算表 + analyze
        let mut book = Workbook::new();
        let sheet = book.add_worksheet();
        sheet.set_name("核算表").unwrap();
        for (c, h) in ["科目", "单位", "月度预定", "月度实绩", "年度计划·当月目标"]
            .iter()
            .enumerate()
        {
            sheet.write_string(0, c as u16, *h).unwrap();
        }
        sheet.write_string(1, 0, "销售额").unwrap();
        sheet.write_string(1, 2, "12000.5").unwrap();
        sheet.write_string(1, 3, "11500.25").unwrap();
        sheet.write_string(1, 4, "12500").unwrap();
        book.save(dir.join("核算表模板.xlsx")).unwrap();
        let (out, code) = run(&[
            "analyze",
            "--db",
            &db,
            "--workspace",
            &workspace.to_string(),
            "--dir",
            &dir.to_string_lossy(),
            "--from",
            "2026-10-01",
            "--to",
            "2026-10-31",
            "--grain",
            "auto",
            "--json",
        ]);
        assert_eq!(code, 0, "{out}");
        let run_id = serde_json::from_str::<serde_json::Value>(&out).unwrap()["data"]["run_id"]
            .as_str()
            .unwrap()
            .to_string();
        Self {
            db,
            dir,
            workspace,
            provider,
            run_id,
            metric_id,
        }
    }
}
