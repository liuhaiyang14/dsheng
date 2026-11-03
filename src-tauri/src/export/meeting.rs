//! 会议版固定 10 页投影（§5）：JSON/Markdown。
//!
//! 1 期间与完整性；2 目标；3 核算预实；4 差异及证据/待确认；5 推移；
//! 6 同比环比/不可比原因；7 已确认追赶目标/行动（缺内容显示“未提供”，不调模型）；
//! 8 本期任务检视；9 下期任务计划；10 证据与风险。不生成 PPTX。

use super::{decimal_of, text_of, ExportData, ExportResult};
use serde_json::{json, Value};

pub const PAGE_TITLES: [&str; 10] = [
    "期间与完整性",
    "目标",
    "核算预实",
    "差异及证据/待确认",
    "推移",
    "同比环比/不可比原因",
    "已确认追赶目标/行动",
    "本期任务检视",
    "下期任务计划",
    "证据与风险",
];

fn task_split(data: &ExportData) -> (Vec<&Value>, Vec<&Value>) {
    let mut review = Vec::new();
    let mut plan = Vec::new();
    for t in &data.tasks {
        match t["target_period"]["to"].as_str() {
            Some(to) if to > data.meta.source_to.as_str() => plan.push(t),
            _ => review.push(t),
        }
    }
    (review, plan)
}

/// 10 页 JSON（页号/标题/内容对象）。
pub fn meeting_json(data: &ExportData) -> ExportResult<Value> {
    let m = &data.meta;
    let (review, plan) = task_split(data);
    let diffs: Vec<Value> = data
        .metrics
        .iter()
        .map(|x| {
            let mut ids: Vec<Value> = Vec::new();
            for k in ["mom", "yoy"] {
                if let Some(arr) = x[k]["evidence_ids"].as_array() {
                    ids.extend(arr.iter().cloned());
                }
            }
            json!({
                "metric_label": x["metric_label"],
                "unit": x["unit"],
                "target_value": x["target_value"],
                "actual_value": x["actual_value"],
                "delta": x["delta"],
                "direction": x["direction"],
                "mom_reason": x["mom"]["reason"],
                "yoy_reason": x["yoy"]["reason"],
                "evidence_ids": ids,
            })
        })
        .collect();
    let catch_up: Vec<Value> = data
        .tasks
        .iter()
        .filter(|t| t["category"] == "revenue" && !t["target_value"].is_null())
        .map(|t| json!({"title": t["title"], "target_value": t["target_value"], "unit": t["unit"], "owner": t["owner"]}))
        .collect();
    let risks: Vec<Value> = data
        .diagnosis
        .as_ref()
        .and_then(|d| d["warnings"].as_array().cloned())
        .unwrap_or_default();
    let pages = vec![
        json!({"page": 1, "title": PAGE_TITLES[0], "content": {
            "workspace": m.workspace_name, "run_id": m.run_id, "revision_no": m.revision_no,
            "as_of": m.as_of, "source_from": m.source_from, "source_to": m.source_to,
            "grain": m.grain, "coverage_status": m.coverage_status,
            "policy": m.policy, "source_manifests": m.source_manifests,
        }}),
        json!({"page": 2, "title": PAGE_TITLES[1], "content": {
            "metrics": data.metrics.iter().map(|x| json!({
                "metric_label": x["metric_label"], "unit": x["unit"], "target_value": x["target_value"],
                "direction": x["direction"],
            })).collect::<Vec<_>>(),
        }}),
        json!({"page": 3, "title": PAGE_TITLES[2], "content": {"metrics": data.metrics}}),
        json!({"page": 4, "title": PAGE_TITLES[3], "content": {"differences": diffs,
            "diagnosis_pending": data.diagnosis.as_ref().and_then(|d| d["open_questions"].clone().into()),
        }}),
        json!({"page": 5, "title": PAGE_TITLES[4], "content": {"series": data.series}}),
        json!({"page": 6, "title": PAGE_TITLES[5], "content": {
            "comparisons": data.metrics.iter().map(|x| json!({
                "metric_label": x["metric_label"], "mom": x["mom"], "yoy": x["yoy"],
            })).collect::<Vec<_>>(),
        }}),
        json!({"page": 7, "title": PAGE_TITLES[6], "content": if catch_up.is_empty() {
            json!({"status": "未提供", "items": []})
        } else {
            json!({"status": "confirmed", "items": catch_up})
        }}),
        json!({"page": 8, "title": PAGE_TITLES[7], "content": {"tasks": review}}),
        json!({"page": 9, "title": PAGE_TITLES[8], "content": {"tasks": plan}}),
        json!({"page": 10, "title": PAGE_TITLES[9], "content": {
            "evidence": data.evidence, "risks": risks,
            "metric_hashes": m.metric_hashes, "base_refs": m.base_refs,
        }}),
    ];
    Ok(json!({
        "schema_version": "1",
        "kind": "meeting_deck",
        "page_count": 10,
        "workspace_id": m.workspace_id,
        "run_id": m.run_id,
        "revision_no": m.revision_no,
        "as_of": m.as_of,
        "pages": pages,
    }))
}

fn md_cell(v: &Value) -> String {
    let t = text_of(v);
    if t.is_empty() {
        "—".into()
    } else {
        t.replace('|', "\\|")
    }
}

/// 10 页 Markdown。
pub fn meeting_markdown(data: &ExportData) -> ExportResult<String> {
    use std::fmt::Write as _;
    let m = &data.meta;
    let (review, plan) = task_split(data);
    let mut s = String::new();
    let _ = writeln!(s, "# 经营会议版（{} ~ {}）", m.source_from, m.source_to);
    let _ = writeln!(s);
    let _ = writeln!(s, "## 1. {}", PAGE_TITLES[0]);
    let _ = writeln!(
        s,
        "- 工作区：{}；run：{}；revision：{}；as_of：{}",
        m.workspace_name, m.run_id, m.revision_no, m.as_of
    );
    let _ = writeln!(
        s,
        "- 期间：{} ~ {}（{}）；覆盖：{}；时区：{}",
        m.source_from, m.source_to, m.grain, m.coverage_status, m.policy.timezone_id
    );
    for src in &m.source_manifests {
        let _ = writeln!(
            s,
            "- 来源：{}{}（{}）",
            src.file_label,
            src.sheet
                .as_ref()
                .map(|x| format!(" / {x}"))
                .unwrap_or_default(),
            &src.content_hash[..src.content_hash.len().min(12)]
        );
    }
    let _ = writeln!(s);
    let _ = writeln!(s, "## 2. {}", PAGE_TITLES[1]);
    let _ = writeln!(s, "| 指标 | 单位 | 目标 | 方向 |");
    let _ = writeln!(s, "| --- | --- | --- | --- |");
    for x in &data.metrics {
        let _ = writeln!(
            s,
            "| {} | {} | {} | {} |",
            md_cell(&x["metric_label"]),
            md_cell(&x["unit"]),
            md_cell(&x["target_value"]),
            md_cell(&x["direction"])
        );
    }
    let _ = writeln!(s);
    let _ = writeln!(s, "## 3. {}", PAGE_TITLES[2]);
    let _ = writeln!(s, "| 指标 | 单位 | 目标 | 实绩 | 差异 |");
    let _ = writeln!(s, "| --- | --- | --- | --- | --- |");
    for x in &data.metrics {
        let _ = writeln!(
            s,
            "| {} | {} | {} | {} | {} |",
            md_cell(&x["metric_label"]),
            md_cell(&x["unit"]),
            md_cell(&x["target_value"]),
            md_cell(&x["actual_value"]),
            md_cell(&x["delta"])
        );
    }
    let _ = writeln!(s);
    let _ = writeln!(s, "## 4. {}", PAGE_TITLES[3]);
    for x in &data.metrics {
        let reasons: Vec<String> = ["mom", "yoy"]
            .iter()
            .filter_map(|k| x[*k]["reason"].as_str().map(|r| format!("{k}:{r}")))
            .collect();
        let _ = writeln!(
            s,
            "- {}：差异 {}；{}",
            md_cell(&x["metric_label"]),
            md_cell(&x["delta"]),
            if reasons.is_empty() {
                "可比".to_string()
            } else {
                format!("待确认/不可比：{}", reasons.join("，"))
            }
        );
    }
    if let Some(d) = &data.diagnosis {
        if let Some(q) = d["open_questions"].as_array() {
            for item in q {
                let _ = writeln!(s, "- 诊断待澄清：{}", md_cell(&item["question"]));
            }
        }
    }
    let _ = writeln!(s);
    let _ = writeln!(s, "## 5. {}", PAGE_TITLES[4]);
    for x in &data.series {
        let buckets: Vec<String> = x["buckets"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .map(text_of)
            .collect();
        let actual: Vec<String> = x["actual"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .map(|v| decimal_of(v).unwrap_or_else(|| "—".into()))
            .collect();
        let _ = writeln!(
            s,
            "- {}（{}）：{}",
            md_cell(&x["metric_label"]),
            md_cell(&x["unit"]),
            buckets
                .iter()
                .zip(actual.iter())
                .map(|(b, a)| format!("{b}={a}"))
                .collect::<Vec<_>>()
                .join("；")
        );
    }
    let _ = writeln!(s);
    let _ = writeln!(s, "## 6. {}", PAGE_TITLES[5]);
    let _ = writeln!(
        s,
        "| 指标 | 环比基期 | 环比绝对差 | 环比变化率 | 同比基期 | 同比绝对差 | 同比变化率 | 原因 |"
    );
    let _ = writeln!(s, "| --- | --- | --- | --- | --- | --- | --- | --- |");
    for x in &data.metrics {
        let base = |c: &Value| {
            if c.is_null() {
                "—".to_string()
            } else {
                format!("{}~{}", text_of(&c["base_from"]), text_of(&c["base_to"]))
            }
        };
        let reasons: Vec<String> = ["mom", "yoy"]
            .iter()
            .filter_map(|k| x[*k]["reason"].as_str().map(|r| format!("{k}:{r}")))
            .collect();
        let _ = writeln!(
            s,
            "| {} | {} | {} | {} | {} | {} | {} | {} |",
            md_cell(&x["metric_label"]),
            base(&x["mom"]),
            md_cell(&x["mom"]["absolute_change"]),
            md_cell(&x["mom"]["change_rate"]),
            base(&x["yoy"]),
            md_cell(&x["yoy"]["absolute_change"]),
            md_cell(&x["yoy"]["change_rate"]),
            if reasons.is_empty() {
                "—".into()
            } else {
                reasons.join("，")
            }
        );
    }
    let _ = writeln!(s);
    let _ = writeln!(s, "## 7. {}", PAGE_TITLES[6]);
    let catch_up: Vec<&Value> = data
        .tasks
        .iter()
        .filter(|t| t["category"] == "revenue" && !t["target_value"].is_null())
        .collect();
    if catch_up.is_empty() {
        let _ = writeln!(s, "未提供");
    } else {
        for t in catch_up {
            let _ = writeln!(
                s,
                "- {}：目标 {} {}（{}）",
                md_cell(&t["title"]),
                md_cell(&t["target_value"]),
                md_cell(&t["unit"]),
                md_cell(&t["owner"])
            );
        }
    }
    let _ = writeln!(s);
    for (idx, (title, tasks)) in [(PAGE_TITLES[7], review), (PAGE_TITLES[8], plan)]
        .into_iter()
        .enumerate()
    {
        let _ = writeln!(s, "## {}. {}", 8 + idx, title);
        if tasks.is_empty() {
            let _ = writeln!(s, "（无）");
        }
        for t in tasks {
            let _ = writeln!(
                s,
                "- [{}] {}（{} · 进度 {} · 截止 {}）",
                md_cell(&t["status"]),
                md_cell(&t["title"]),
                md_cell(&t["owner"]),
                md_cell(&t["progress"]),
                md_cell(&t["due_date"])
            );
            for step in t["steps"].as_array().cloned().unwrap_or_default() {
                let _ = writeln!(
                    s,
                    "  - 步骤：{}（{}，{} ~ {}）",
                    md_cell(&step["action"]),
                    md_cell(&step["status"]),
                    md_cell(&step["start_date"]),
                    md_cell(&step["due_date"])
                );
            }
        }
        let _ = writeln!(s);
    }
    let _ = writeln!(s, "## 10. {}", PAGE_TITLES[9]);
    for e in &data.evidence {
        let _ = writeln!(
            s,
            "- {}：{} / {} / {}",
            md_cell(&e["evidence_id"]),
            md_cell(&e["file_label"]),
            md_cell(&e["sheet"]),
            md_cell(&e["cell_or_row"])
        );
    }
    if let Some(d) = &data.diagnosis {
        for w in d["warnings"].as_array().cloned().unwrap_or_default() {
            let _ = writeln!(s, "- 风险：{}", md_cell(&w["summary"]));
        }
    }
    for h in &m.metric_hashes {
        let _ = writeln!(
            s,
            "- 指标 {} v{}：{}",
            h.metric_key,
            h.version,
            &h.semantic_hash[..12.min(h.semantic_hash.len())]
        );
    }
    Ok(s)
}
