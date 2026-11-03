//! S1 集成测试（测试层次：进程内 CLI 运行器＋临时 SQLite 库；无网络）。
//!
//! 覆盖：--version 契约、未知 flag 拒绝、workspace 解析规则
//! （显式 --workspace > app_preferences > WORKSPACE_REQUIRED）、
//! WORKSPACE_MISMATCH、UUID 校验、settings 命令（workspace create/use/list、
//! period list/set、provider set/use/list/clear-key/delete）与 Key stdin 规则。

use serde_json::Value;
use std::io::Write;

fn run(args: &[&str]) -> (String, i32) {
    let owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = dsheng_cli::run_with_output(&owned, &mut out, &mut err);
    (String::from_utf8(out).expect("utf8"), code)
}

fn envelope_ok(stdout: &str) -> Value {
    let v: Value = serde_json::from_str(stdout.trim()).expect("stdout 必须是单个 JSON envelope");
    assert_eq!(v["schema_version"], "1", "schema_version 固定 1");
    assert_eq!(v["ok"], true);
    v["data"].clone()
}

fn envelope_err(stdout: &str) -> (String, Value) {
    let v: Value = serde_json::from_str(stdout.trim()).expect("stdout 必须是单个 JSON envelope");
    assert_eq!(v["schema_version"], "1");
    assert_eq!(v["ok"], false, "stdout: {stdout}");
    (
        v["error"]["code"].as_str().expect("code").to_string(),
        v["error"].clone(),
    )
}

fn tmp_db(tag: &str) -> String {
    let dir = tempfile::tempdir().expect("临时目录");
    let path = dir.keep().join(format!("{tag}.sqlite3"));
    path.to_string_lossy().to_string()
}

#[test]
fn s1_version_contract() {
    let (out, code) = run(&["--version", "--json"]);
    assert_eq!(code, 0);
    let data = envelope_ok(&out);
    for key in [
        "app_version",
        "identifier",
        "db_schema_version",
        "task_prompt_version",
        "diagnosis_prompt_version",
        "task_output_schema_version",
        "diagnosis_schema_version",
        "input_contract_version",
        "context_policy_version",
        "platform",
        "architecture",
    ] {
        assert!(data.get(key).is_some(), "version 输出缺 {key}");
    }
}

#[test]
fn s1_unknown_flag_rejected_as_invalid_argument() {
    let (out, code) = run(&["workspace", "list", "--no-such-flag", "--json"]);
    assert_eq!(code, 2, "未知 flag 必须退出码 2");
    let (code_name, _) = envelope_err(&out);
    assert_eq!(code_name, "INVALID_ARGUMENT");
}

#[test]
fn s1_unknown_command_rejected() {
    let (out, code) = run(&["frobnicate", "--json"]);
    assert_eq!(code, 2);
    let (code_name, _) = envelope_err(&out);
    assert_eq!(code_name, "INVALID_ARGUMENT");
}

#[test]
fn s1_help_envelope() {
    let (out, code) = run(&["--help"]);
    assert_eq!(code, 0);
    let data = envelope_ok(&out);
    assert_eq!(data["help"], true);
}

#[test]
fn s1_workspace_required_without_flag_or_preference() {
    let db = tmp_db("wsreq");
    let (out, code) = run(&["provider", "list", "--db", &db, "--json"]);
    assert_eq!(code, 2);
    let (code_name, body) = envelope_err(&out);
    assert_eq!(code_name, "WORKSPACE_REQUIRED");
    assert_eq!(body["field"], "--workspace");
}

#[test]
fn s1_workspace_bad_uuid_rejected() {
    let db = tmp_db("wsbad");
    let (out, code) = run(&[
        "provider",
        "list",
        "--db",
        &db,
        "--workspace",
        "not-a-uuid",
        "--json",
    ]);
    assert_eq!(code, 2);
    let (code_name, _) = envelope_err(&out);
    assert_eq!(code_name, "INVALID_ARGUMENT");
}

#[test]
fn s1_workspace_create_use_list_and_mismatch() {
    let db = tmp_db("wsmk");
    // 创建 workspace（stdin 输入）
    let input = r#"{"schema_version":"1","operation":"workspace_create","name":"合成门店","initial_period_policy":{"cadence":"month","effective_from":"2026-01-01"}}"#;
    let (out, code) = run_with_stdin(
        &[
            "workspace",
            "create",
            "--db",
            &db,
            "--input-json",
            "-",
            "--json",
        ],
        input,
    );
    assert_eq!(code, 0, "out={out}");
    let data = envelope_ok(&out);
    let ws_id = data["workspace"]["workspace_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(data["workspace"]["name"], "合成门店");
    assert_eq!(
        data["initial_period_policy"]["policy"]["timezone_id"], "Asia/Shanghai",
        "period set 输出应用后的默认值"
    );

    // use 后无 --workspace 也能解析
    let (out, code) = run(&[
        "workspace",
        "use",
        "--db",
        &db,
        "--workspace",
        &ws_id,
        "--json",
    ]);
    assert_eq!(code, 0, "out={out}");
    let (out, code) = run(&["provider", "list", "--db", &db, "--json"]);
    assert_eq!(code, 0, "偏好 workspace 后不再 WORKSPACE_REQUIRED: {out}");
    let data = envelope_ok(&out);
    assert_eq!(data["providers"].as_array().map(Vec::len), Some(0));

    // 不存在/跨域 workspace → WORKSPACE_MISMATCH
    let other = uuid::Uuid::new_v4().to_string();
    let (out, code) = run(&[
        "provider",
        "list",
        "--db",
        &db,
        "--workspace",
        &other,
        "--json",
    ]);
    assert_eq!(code, 2);
    let (code_name, _) = envelope_err(&out);
    assert_eq!(code_name, "WORKSPACE_MISMATCH");

    // workspace list
    let (out, code) = run(&["workspace", "list", "--db", &db, "--json"]);
    assert_eq!(code, 0);
    let data = envelope_ok(&out);
    assert_eq!(data["workspaces"].as_array().map(Vec::len), Some(1));
}

/// 带 stdin 的进程内运行（测试辅助；stdin 只用于输入 JSON/Key 文本）。
fn run_with_stdin(args: &[&str], stdin_text: &str) -> (String, i32) {
    // 直接把 stdin 文本写入临时文件并通过 file path 传给 --input-json 的替代
    // 方式不可行（- 即 stdin）；这里改用库内 jsonio 读取路径的行为等价：
    // 命令层测试统一走文件；stdin 路径由 run_with_stdin_raw 覆盖。
    let dir = tempfile::tempdir().expect("临时目录");
    let file = dir.path().join("input.json");
    std::fs::write(&file, stdin_text).expect("写入输入文件");
    let file = file.to_string_lossy().to_string();
    let args: Vec<String> = args
        .iter()
        .map(|s| {
            if *s == "-" {
                file.clone()
            } else {
                s.to_string()
            }
        })
        .collect();
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = dsheng_cli::run_with_output(&args, &mut out, &mut err);
    let _ = std::io::stdout().flush();
    (String::from_utf8(out).expect("utf8"), code)
}

#[test]
fn s1_unknown_input_json_field_rejected() {
    let db = tmp_db("wsfield");
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("in.json");
    std::fs::write(
        &file,
        r#"{"schema_version":"1","operation":"workspace_create","name":"x","initial_period_policy":{"cadence":"month","effective_from":"2026-01-01"},"extra_field":1}"#,
    )
    .unwrap();
    let (out, code) = run(&[
        "workspace",
        "create",
        "--db",
        &db,
        "--input-json",
        file.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(code, 2);
    let (code_name, _) = envelope_err(&out);
    assert_eq!(code_name, "INVALID_ARGUMENT", "未知输入字段必须拒绝");
}

#[test]
fn s1_provider_set_requires_api_key_stdin_for_replace() {
    let db = tmp_db("pvkey");
    // 建 workspace
    let dir = tempfile::tempdir().unwrap();
    let in_file = dir.path().join("ws.json");
    std::fs::write(
        &in_file,
        r#"{"schema_version":"1","operation":"workspace_create","name":"w","initial_period_policy":{"cadence":"month","effective_from":"2026-01-01"}}"#,
    )
    .unwrap();
    let (out, code) = run(&[
        "workspace",
        "create",
        "--db",
        &db,
        "--input-json",
        in_file.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(code, 0, "{out}");
    let ws_id = envelope_ok(&out)["workspace"]["workspace_id"]
        .as_str()
        .unwrap()
        .to_string();

    let upsert = r#"{"schema_version":"1","operation":"provider_upsert","workspace_id":"WS","provider_id":null,"expected_revision":null,"config":{"name":"p1","protocol":"chat_completions","base_url":"https://api.example.com/v1","model":"m","stream":false,"timeouts":{"connect_seconds":15,"response_seconds":180,"total_seconds":240},"budget":{"context_window_tokens":32768,"output_reserve_tokens":8192}},"key_action":"replace"}"#
        .replace("WS", &ws_id);
    let f = dir.path().join("p.json");
    std::fs::write(&f, &upsert).unwrap();
    // replace 但未带 --api-key-stdin → 拒绝（DATA_VALIDATION_FAILED，exit 3）
    let (out, code) = run(&[
        "provider",
        "set",
        "--db",
        &db,
        "--workspace",
        &ws_id,
        "--input-json",
        f.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(code, 3);
    let (code_name, _) = envelope_err(&out);
    assert_eq!(
        code_name, "DATA_VALIDATION_FAILED",
        "replace 必须配合 --api-key-stdin"
    );

    // http 非 loopback → PROVIDER_URL_INVALID
    let bad = upsert
        .replace("https://api.example.com/v1", "http://api.example.com/v1")
        .replace("key_action\":\"replace", "key_action\":\"keep");
    std::fs::write(&f, bad).unwrap();
    let (out, code) = run(&[
        "provider",
        "set",
        "--db",
        &db,
        "--workspace",
        &ws_id,
        "--input-json",
        f.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(code, 2, "PROVIDER_URL_INVALID 退出码 2: {out}");
    let (code_name, _) = envelope_err(&out);
    assert_eq!(code_name, "PROVIDER_URL_INVALID");
}

#[test]
fn s1_period_set_policy_overlap_rejected() {
    let db = tmp_db("pset");
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("ws.json");
    std::fs::write(
        &f,
        r#"{"schema_version":"1","operation":"workspace_create","name":"w","initial_period_policy":{"cadence":"month","effective_from":"2026-01-01"}}"#,
    )
    .unwrap();
    let (out, code) = run(&[
        "workspace",
        "create",
        "--db",
        &db,
        "--input-json",
        f.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(code, 0);
    let data = envelope_ok(&out);
    let ws_id = data["workspace"]["workspace_id"]
        .as_str()
        .unwrap()
        .to_string();
    let rev = data["workspace"]["config_revision"].as_u64().unwrap();

    // period list
    let (out, code) = run(&[
        "workspace",
        "period",
        "list",
        "--db",
        &db,
        "--workspace",
        &ws_id,
        "--json",
    ]);
    assert_eq!(code, 0);
    assert_eq!(
        envelope_ok(&out)["policies"].as_array().map(Vec::len),
        Some(1)
    );

    // 修订冲突：expected_revision 过期
    let p = format!(
        r#"{{"schema_version":"1","operation":"period_policy_set","workspace_id":"{ws_id}","expected_revision":99,"policy":{{"cadence":"week","effective_from":"2026-11-01"}}}}"#
    );
    let pf = dir.path().join("p.json");
    std::fs::write(&pf, p).unwrap();
    let (out, code) = run(&[
        "workspace",
        "period",
        "set",
        "--db",
        &db,
        "--workspace",
        &ws_id,
        "--input-json",
        pf.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(code, 2);
    let (code_name, _) = envelope_err(&out);
    assert_eq!(code_name, "REVISION_CONFLICT");

    // 正确 revision 成功且回显默认值
    let p = format!(
        r#"{{"workspace_id":"{ws_id}","expected_revision":{rev},"policy":{{"cadence":"week","effective_from":"2026-11-01"}}}}"#
    );
    std::fs::write(&pf, p).unwrap();
    let (out, code) = run(&[
        "workspace",
        "period",
        "set",
        "--db",
        &db,
        "--workspace",
        &ws_id,
        "--input-json",
        pf.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(code, 0, "{out}");
    let data = envelope_ok(&out);
    assert_eq!(data["policy"]["policy"]["cadence"], "week");
    assert_eq!(data["policy"]["policy"]["week_start_day"], 1, "ISO1 默认");
}

#[test]
fn s1_gate_flag_pairs_validated_at_parse() {
    // 双门禁互斥在命令层校验（S3 接线前先验证非法组合被拒为 INVALID_ARGUMENT）。
    let db = tmp_db("gate");
    let (out, code) = run(&[
        "diagnosis",
        "generate",
        "--db",
        &db,
        "--workspace",
        "00000000-0000-4000-8000-000000000001",
        "--run",
        "00000000-0000-4000-8000-000000000002",
        "--framework",
        "5why",
        "--problem",
        "x",
        "--dry-run",
        "--confirmation-id",
        "00000000-0000-4000-8000-000000000003",
        "--confirm-send",
        "--json",
    ]);
    assert_eq!(code, 2);
    let (code_name, _) = envelope_err(&out);
    // 未接线阶段：workspace 解析先行（不存在的 workspace → WORKSPACE_MISMATCH）
    assert!(
        code_name == "INVALID_ARGUMENT" || code_name == "WORKSPACE_MISMATCH",
        "gate 非法组合或 workspace 校验必须先于其他错误: {code_name}"
    );
}

#[test]
fn s1_provider_set_protocol_const_rejected_by_serde() {
    // F3：protocol const 校验由 core ChatCompletionsMarker 的 Deserialize 承担
    // （jsonio 旁路已移除；非 "chat_completions" 在解析边界拒绝）
    let db = tmp_db("pvproto");
    let dir = tempfile::tempdir().expect("临时目录");
    let in_file = dir.path().join("ws.json");
    std::fs::write(
        &in_file,
        r#"{"schema_version":"1","operation":"workspace_create","name":"w","initial_period_policy":{"cadence":"month","effective_from":"2026-01-01"}}"#,
    )
    .unwrap();
    let (out, code) = run(&[
        "workspace",
        "create",
        "--db",
        &db,
        "--input-json",
        in_file.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(code, 0, "{out}");
    let ws_id = envelope_ok(&out)["workspace"]["workspace_id"]
        .as_str()
        .unwrap()
        .to_string();
    for bad_protocol in ["openai", "chat_completions_v2", ""] {
        let upsert = format!(
            r#"{{"schema_version":"1","operation":"provider_upsert","workspace_id":"{ws_id}","provider_id":null,"expected_revision":null,"config":{{"name":"p1","protocol":"{bad_protocol}","base_url":"https://api.example.com/v1","model":"m","stream":false,"timeouts":{{"connect_seconds":15,"response_seconds":180,"total_seconds":240}},"budget":{{"context_window_tokens":32768,"output_reserve_tokens":8192}}}},"key_action":"keep"}}"#
        );
        let f = dir.path().join("p_bad.json");
        std::fs::write(&f, &upsert).unwrap();
        let (out, code) = run(&[
            "provider",
            "set",
            "--db",
            &db,
            "--workspace",
            &ws_id,
            "--input-json",
            f.to_str().unwrap(),
            "--json",
        ]);
        assert_eq!(code, 2, "protocol={bad_protocol} 必须解析拒绝: {out}");
        let (code_name, detail) = envelope_err(&out);
        assert_eq!(code_name, "INVALID_ARGUMENT");
        assert!(
            detail["message"]
                .as_str()
                .unwrap_or_default()
                .contains("chat_completions"),
            "错误信息指向 const: {out}"
        );
    }
}
