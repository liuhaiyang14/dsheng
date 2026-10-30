#!/usr/bin/env python3
"""Offline planning checks. Run: python3 tools/validate_contracts.py

Requires jsonschema >= 4.18 and referencing. Reads only repository contracts and
prompt resources; writes nothing. Synthetic reports are Schema fixtures, never
Provider responses or fabricated model history. No product logic is executed.
"""
from collections import Counter
from copy import deepcopy
from decimal import Decimal
from datetime import date
import hashlib
import json
from pathlib import Path
import re
import sys

try:
    from jsonschema import Draft202012Validator, FormatChecker
    from referencing import Registry, Resource
    from referencing.exceptions import NoSuchResource
except ImportError as exc:
    raise SystemExit("Install jsonschema >= 4.18 before running this validator.") from exc

ROOT = Path(__file__).resolve().parents[1]
CONTRACTS = ROOT / "contracts/v1"
PROMPTS = ROOT / "crates/dsheng-core/prompts/v1"
UUID = "00000000-0000-4000-8000-000000000001"
SOURCE = "00000000-0000-4000-8000-000000000010"
MANUAL = "00000000-0000-4000-8000-000000000011"
EVIDENCE = f"cell:{SOURCE}:数据:B2"
METRIC = "00000000-0000-4000-8000-000000000020"
HASH = "a" * 64


def require(condition, message):
    if not condition:
        raise ValueError(message)


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, f"Duplicate JSON key: {key}")
        result[key] = value
    return result


def read_json(path):
    def reject_constant(value):
        raise ValueError(f"Non-finite JSON number: {value}")
    return json.loads(path.read_text(encoding="utf-8"), object_pairs_hook=unique_object,
                      parse_float=Decimal, parse_constant=reject_constant)


def no_remote_resource(uri):
    raise NoSuchResource(ref=uri)


def unique_ids(rows, key, label):
    values = [row[key] for row in rows]
    require(len(values) == len(set(values)), f"Duplicate {label}")


PERIOD_RE = re.compile(r"^(\d{4}-\d{2}-\d{2})/(\d{4}-\d{2}-\d{2})$")
CELL_RE = re.compile(r"^([A-Z]{1,3})([1-9][0-9]*)$")


def valid_period(value):
    """Require an ISO closed date interval with real calendar dates."""
    if value is None:
        return True
    match = PERIOD_RE.fullmatch(value) if isinstance(value, str) else None
    if not match:
        return False
    try:
        start, end = (date.fromisoformat(part) for part in match.groups())
    except ValueError:
        return False
    return start <= end


def valid_cell_reference(value):
    """Require an Excel A1 coordinate within XLSX limits (A1:XFD1048576)."""
    match = CELL_RE.fullmatch(value) if isinstance(value, str) else None
    if not match:
        return False
    column, row = match.groups()
    number = 0
    for char in column:
        number = number * 26 + ord(char) - ord("A") + 1
    return number <= 16384 and int(row) <= 1048576


def validate_input_domains(value):
    """Cross-field input checks that Draft 2020-12 cannot express alone."""
    errors = []
    period = value.get("period", {})
    for key, item in period.items():
        if key.endswith("_from") or key.endswith("_to"):
            try:
                date.fromisoformat(item)
            except (TypeError, ValueError):
                errors.append(f"period.{key} is not a real ISO date")
    for index, item in enumerate(value.get("manual_inputs", [])):
        if not valid_period(item.get("period")):
            errors.append(f"manual_inputs[{index}].period is not a valid closed date interval")
    for index, item in enumerate(value.get("source_conflicts", [])):
        if not valid_period(item.get("period")):
            errors.append(f"source_conflicts[{index}].period is not a valid closed date interval")
    for index, record in enumerate(value.get("source_records", [])):
        if not valid_cell_reference(record.get("cell_or_row")):
            errors.append(f"source_records[{index}].cell_or_row is outside XLSX limits")
        mapping = record.get("values", {}).get("mapping")
        if mapping and not valid_period(mapping.get("period")):
            errors.append(f"source_records[{index}].values.mapping.period is not a valid closed date interval")
    return errors


def walk_refs(value):
    if isinstance(value, dict):
        if "$ref" in value:
            yield value["$ref"]
        for child in value.values():
            yield from walk_refs(child)
    elif isinstance(value, list):
        for child in value:
            yield from walk_refs(child)


def changed(value, path, replacement):
    value = deepcopy(value)
    parent = value
    for part in path[:-1]:
        parent = parent[part]
    parent[path[-1]] = replacement
    return value


def base_input(workflow):
    unit = {"base_unit": "元", "currency": "CNY", "dimension": "money", "tax_basis": "exclusive"}
    unit_text = json.dumps(unit, ensure_ascii=False, sort_keys=True, separators=(",", ":"))
    definition = {"metric_id": METRIC, "metric_key": "sales_net", "version": 1,
        "semantic_hash": HASH, "label": "合成销售额", "semantic_type": "flow",
        "unit_signature": unit, "direction": "up", "target_constraint_scope": "monthly_revenue_floor",
        "revenue_semantics": True, "aggregation": "sum_disjoint", "formula": None,
        "required_inputs": [], "denominator_policy": "positive_only",
        "comparison_policy": {"positive_base": "relative_change", "zero_or_negative_base": "absolute_only"},
        "effective_from": "2026-01-01", "scope_key": "synthetic-total"}
    metric = {"metric_id": METRIC, "label": "合成销售额", "unit": unit_text, "direction": "up",
        "target_constraint_scope": "monthly_revenue_floor", "explicit_target": None,
        "annual_allocated_target": None, "prior_year_actual": None, "target": Decimal("110.25"),
        "actual": Decimal("100.25"), "delta": Decimal("-10"), "mom": None, "yoy": None,
        "evidence_ids": [EVIDENCE]}
    data = {"request": {"input_schema_version": "2", "run_id": UUID, "analysis_revision_no": 1,
        "analysis_input_fingerprint": HASH, "prompt_version": "2.0", "workflow": workflow,
        "schema_version": "2" if workflow == "task" else "3"},
        "workspace": {"workspace_id": UUID, "industry": None, "cadence": "month",
            "timezone": "Asia/Shanghai", "metric_definitions": [definition]}, "metrics": [metric],
        "source_manifest": [{"source_id": SOURCE, "file_label": "合成数据", "sheet": "数据",
            "selected_range": "A1:B2", "content_hash": HASH, "selection_hash": HASH}],
        "source_records": [{"evidence_id": EVIDENCE, "source_id": SOURCE, "file_label": "合成数据",
            "sheet": "数据", "cell_or_row": "B2", "values": {
                "cell_type": "number", "raw_text": "100.25", "typed_value": "100.25", "formula": None,
                "value_state": "valid", "number_format": "0.00", "merged_range": None,
                "mapping": {"metric_id": METRIC, "field_key": "actual", "measure_role": "actual",
                    "period": "2026-09-01/2026-09-30", "scope_key": "synthetic-total", "unit": unit_text}}}],
        "manual_inputs": [], "missing_or_anomalous_inputs": [], "source_conflicts": [],
        "period": {"source_from": "2026-09-01", "source_to": "2026-09-30", "period_policy_version": "1"}}
    if workflow == "task":
        data["period"].update(target_from="2026-10-01", target_to="2026-10-31", target_grain="month")
        data.update(business_review={"summary": "合成Schema输入", "prior_task_outcomes": []},
            task_skill_context={"rules_version": "2.0", "required_fields": ["task_key", "title", "summary", "category", "metric_id", "source_period", "target_period", "baseline", "target_value", "unit", "owner", "planned_start", "due_date", "steps", "resource_need", "success_criteria", "evidence_ids", "assumptions", "needs_review", "smart_check"],
                "category_options": ["revenue", "cost", "efficiency", "other"]},
            optional_diagnosis_reference=None, user_context=None,
            output_contract={"schema": "task-output-v2", "schema_version": "2"})
    else:
        data["request"].update(report_id=None, generation_session_id=UUID, round_no=1,
            framework="5why", user_confirmed_continue=False, context_policy_version="2")
        data.update(scope={"problem_statement": "合成差异的待确认原因", "focus_metric_ids": [METRIC],
            "framework": "5why", "open_question_ids": []}, user_message="", continuation_input=None,
            session_state={"accepted_facts": [], "prior_report_summary": None,
                "unresolved_questions": [], "pending_clarification": None},
            output_contract={"schema": "diagnosis-turn-5why-v3", "schema_version": "3"})
    return data


def report_fixture(data, framework):
    """Pure schema object; no model output or semantic acceptance."""
    findings = []
    dimensions = {"5why": [None], "5w": ["What", "Why", "Where", "When", "Who"],
                  "5w2h": ["What", "Why", "Who", "When", "Where", "How", "How much"]}
    for index, dimension in enumerate(dimensions[framework], 1):
        finding = {"finding_id": f"F{index}", "question": "合成待确认问题", "answer": "未知",
            "fact_status": "unknown", "evidence_ids": [], "confidence": "unknown", "status": "needs_confirmation"}
        if framework == "5why":
            finding.update(level=index, reasoning_summary="Schema fixture only", validation_question=None)
        else:
            finding.update(dimension=dimension, missing_information=[], clarification_questions=[])
            if framework == "5w2h":
                finding["kind"] = "observed"
        findings.append(finding)
    return {"schema_version": "3", "report_id": "synthetic-schema-report", "generation_session_id": UUID,
        "workspace_id": UUID, "run_id": UUID, "framework": framework,
        "scope": {"problem_statement": data["scope"]["problem_statement"], "focus_metric_ids": [METRIC]},
        "period": {"from": "2026-09-01", "to": "2026-09-30", "period_policy_version": "1"},
        "revision_no": 1, "summary": "纯合成Schema对象，不是Provider响应",
        "source_manifest": deepcopy(data["source_manifest"]), "findings": findings,
        "facts_and_hypotheses": [], "evidence_refs": [], "clarification_questions": [], "warnings": [],
        "stop_reason": "待补充", "can_continue": True, "created_at": "2026-10-30T00:00:00Z",
        "updated_at": "2026-10-30T00:00:00Z", "analysis_revision_no": 1, "analysis_input_fingerprint": HASH}


def input_cases():
    cases = []
    def add(name, schema, value, valid=True):
        cases.append((name, schema, deepcopy(value), valid))
    task, diagnosis = base_input("task"), base_input("diagnosis")
    for workflow, initial in [("task", task), ("diagnosis", diagnosis)]:
        schema = f"dsheng-{workflow}-input-v2"
        add(f"{workflow}: initial", schema, initial)
        for name, path, value in [
            ("bad date", ["period", "source_from"], "2026-02-30"),
            ("bad metric UUID", ["metrics", 0, "metric_id"], "sales_net"),
            ("bad source hash", ["source_manifest", 0, "selection_hash"], "fixture-v1"),
            ("model number is not local string", ["metrics", 0, "actual"], "100.25"),
            ("floor requires revenue", ["workspace", "metric_definitions", 0, "revenue_semantics"], False),
            ("floor cannot be down", ["workspace", "metric_definitions", 0, "direction"], "down"),
        ]:
            add(f"{workflow}: {name}", schema, changed(initial, path, value), False)
        candidate = {"value": Decimal("120.125"), "unit": initial["metrics"][0]["unit"],
            "evidence_ids": ["E1"], "comparable": True, "conflict_status": "resolved", "resolution_id": UUID}
        with_candidate = changed(initial, ["metrics", 0, "explicit_target"], candidate)
        add(f"{workflow}: resolved target", schema, with_candidate)
        add(f"{workflow}: target missing resolution", schema,
            changed(with_candidate, ["metrics", 0, "explicit_target", "resolution_id"], None), False)
        conflict = {"conflict_id": UUID, "metric_id": METRIC, "measure_role": "actual",
            "period": "2026-09-01/2026-09-30", "scope_key": "synthetic-total", "unit": initial["metrics"][0]["unit"],
            "candidate_values": [{"value": 100, "evidence_id": EVIDENCE, "source_label": "合成甲"},
                                 {"value": 101, "evidence_id": f"cell:{SOURCE}:数据:C2", "source_label": "合成乙"}],
            "resolution_status": "resolved", "resolution_id": UUID,
            "selected_evidence_id": "E1", "resolution_note": "合成来源选择"}
        with_conflict = changed(initial, ["source_conflicts"], [conflict])
        add(f"{workflow}: resolved conflict structure", schema, with_conflict)
        add(f"{workflow}: conflict missing selected evidence", schema,
            changed(with_conflict, ["source_conflicts", 0, "selected_evidence_id"], None), False)
        add(f"{workflow}: unresolved cannot retain resolution", schema,
            changed(with_conflict, ["source_conflicts", 0, "resolution_status"], "unresolved"), False)
        manual = {
            "input_id": "00000000-0000-4000-8000-000000000012",
            "metric_id": METRIC,
            "value": "1",
            "unit": initial["metrics"][0]["unit"],
            "confirmed": True,
            "source_note": "合成手工值",
            "evidence_ids": ["manual:00000000-0000-4000-8000-000000000012:r1"],
            "input_revision": 1,
            "measure_role": "actual",
            "period": "2026-09-01/2026-09-30",
            "scope_key": "synthetic-total",
            "raw_text": "1",
        }
        with_manual = deepcopy(initial)
        with_manual["manual_inputs"] = [manual]
        add(f"{workflow}: valid manual period", schema, with_manual)
        add(f"{workflow}: invalid manual calendar date", schema,
            changed(with_manual, ["manual_inputs", 0, "period"], "2026-02-30/2026-03-01"), False)
        add(f"{workflow}: source cell beyond XFD", schema,
            changed(initial, ["source_records", 0, "cell_or_row"], "XFE1"), False)
        add(f"{workflow}: mapping invalid period", schema,
            changed(initial, ["source_records", 0, "values", "mapping", "period"], "2026-02-30/2026-03-01"), False)

    schema = "dsheng-diagnosis-input-v2"
    for framework in ["5why", "5w", "5w2h"]:
        initial = deepcopy(diagnosis)
        initial["request"]["framework"] = initial["scope"]["framework"] = framework
        initial["output_contract"]["schema"] = f"diagnosis-turn-{framework}-v3"
        if framework != "5why":
            add(f"diagnosis: initial {framework}", schema, initial)
        wrong = "5w" if framework == "5why" else "5why"
        add(f"diagnosis: mismatched scope {framework}", schema,
            changed(initial, ["scope", "framework"], wrong), False)
        add(f"diagnosis: mismatched output {framework}", schema,
            changed(initial, ["output_contract", "schema"], f"diagnosis-turn-{wrong}-v3"), False)
        continuation = deepcopy(initial)
        continuation["request"].update(round_no=2, user_confirmed_continue=True, report_id="synthetic-schema-report")
        continuation["user_message"] = "补充合成Schema输入。"
        continuation["continuation_input"] = {"mode": "append", "answer_to_turn_id": None,
            "selected_open_question_ids": [], "user_message": continuation["user_message"]}
        continuation["session_state"]["prior_report_summary"] = report_fixture(initial, framework)
        add(f"diagnosis: nonnull full report {framework}", schema, continuation)
        missing = deepcopy(continuation)
        del missing["session_state"]["prior_report_summary"]["created_at"]
        add(f"diagnosis: incomplete report {framework}", schema, missing, False)
        if framework == "5why":
            saved_continuation = continuation
    no_report = deepcopy(saved_continuation)
    no_report["request"]["report_id"] = None
    no_report["session_state"]["prior_report_summary"] = None
    add("diagnosis: continuation without report", schema, no_report)
    for name, path, value in [
        ("continuation needs confirmation", ["request", "user_confirmed_continue"], False),
        ("continuation requires input", ["continuation_input"], None),
        ("append cannot target pending", ["continuation_input", "answer_to_turn_id"], "pending-1"),
    ]:
        add(f"diagnosis: {name}", schema, changed(no_report, path, value), False)
    pending = {"turn_id": "pending-1", "user_message": "合成含糊补充",
        "clarification_questions": [{"question_id": "Q-pending", "question": "请明确期间",
            "why_needed": "范围需明确", "evidence_ids": []}], "redirect_message": None, "stop_reason": "范围待确认"}
    restate = deepcopy(no_report)
    restate["continuation_input"].update(mode="restate", answer_to_turn_id="pending-1")
    restate["session_state"]["pending_clarification"] = pending
    add("diagnosis: restate with pending", schema, restate)
    add("diagnosis: restate needs target", schema,
        changed(restate, ["continuation_input", "answer_to_turn_id"], None), False)
    add("diagnosis: restate needs pending", schema,
        changed(restate, ["session_state", "pending_clarification"], None), False)
    add("diagnosis: revised first text", schema, changed(diagnosis, ["user_message"], "用于新报告的完整更正补充。"))
    add("diagnosis: first cannot carry report", schema,
        changed(diagnosis, ["session_state", "prior_report_summary"], report_fixture(diagnosis, "5why")), False)
    add("diagnosis: text length limit", schema, changed(diagnosis, ["user_message"], "字" * 8001), False)
    return cases


def validate_collections(documents, counts):
    catalog = documents[CONTRACTS / "metric-catalog.json"]
    metrics = catalog["metrics"]
    unique_ids(metrics, "metric_key", "metric_key")
    required = {"metric_key", "label", "semantic_type", "unit", "direction", "target_constraint_scope",
        "aggregation", "formula", "required_inputs", "denominator_policy", "comparison_policy", "version",
        "effective_from", "revenue_semantics"}
    for metric in metrics:
        require(required <= metric.keys(), f"Catalog fields missing: {metric['metric_key']}")
        require(re.fullmatch(r"[a-z][a-z0-9_]{0,63}", metric["metric_key"]), "Invalid metric key")
        if metric["target_constraint_scope"] == "monthly_revenue_floor":
            require(metric["direction"] == "up" and metric["revenue_semantics"] is True,
                    "Catalog floor requires up-direction revenue")
    counts["catalog_metric_entries_structurally_checked"] = len(metrics)
    data = documents[CONTRACTS / "data-period-cases.json"]["cases"]
    unique_ids(data, "case_id", "data case IDs")
    for case in data:
        require(set(case) == {"case_id", "operation", "input", "expected"}, "Invalid data case fields")
        require(isinstance(case["input"], dict) and isinstance(case["expected"], dict), "Invalid data case objects")
    counts["data_cases_structurally_checked_not_executed"] = len(data)
    tasks = documents[CONTRACTS / "task-state-cases.json"]
    for group in ["schema_validation_cases", "domain_acceptance_cases"]:
        unique_ids(tasks[group], "id", group)
    for case in tasks["domain_acceptance_cases"]:
        require({"id", "test_layer", "given", "when", "expected"} <= case.keys(), "Invalid task case fields")
        require(isinstance(case["given"], dict) and isinstance(case["expected"], dict), "Invalid task case objects")
    counts["task_domain_cases_structurally_checked_not_executed"] = len(tasks["domain_acceptance_cases"])
    cli = documents[CONTRACTS / "cli-surface.json"]
    unique_ids(cli["commands"], "command", "CLI commands")
    for command in cli["commands"]:
        require(command["provider_calls_maximum"] in (0, 1), "Unexpected automatic Provider call count")
        require(command["syntax"].startswith("dsheng "), "Unexpected CLI binary")
    counts["cli_commands_structurally_checked_not_executed"] = len(cli["commands"])
    counts["cli_error_codes_parsed"] = len(cli["errors"])


def validate_hashes(manifest, counts):
    paths = set()
    for bundle in manifest["prompt_bundles"].values():
        paths.update(bundle["files"])
        paths.add(bundle["response_schema"])
        input_path = (PROMPTS / bundle["input_schema"]).resolve()
        require(input_path.is_relative_to(ROOT) and input_path.is_file(), "Invalid bundled input path")
    paths.add(manifest["diagnosis_report_schema"])
    recorded = manifest["current_resource_hashes"]
    require(set(recorded) == paths, "Current resource hash coverage differs from bundle files")
    failures = []
    for name, expected in recorded.items():
        file = (PROMPTS / name).resolve()
        require(file.is_relative_to(ROOT), "Resource path escapes repository")
        if hashlib.sha256(file.read_bytes()).hexdigest() != expected:
            failures.append(str(file.relative_to(ROOT)))
    inputs = manifest["current_input_contract_hashes"]
    expected_inputs = {str((PROMPTS / b["input_schema"]).resolve().relative_to(ROOT))
                       for b in manifest["prompt_bundles"].values()}
    require(set(inputs) == expected_inputs, "Current input hash coverage differs")
    for name, expected in inputs.items():
        file = (ROOT / name).resolve()
        require(file.is_relative_to(ROOT), "Input hash path escapes repository")
        if hashlib.sha256(file.read_bytes()).hexdigest() != expected:
            failures.append(name)
    counts["current_resource_hashes_checked"] = len(recorded)
    counts["current_input_hashes_checked"] = len(inputs)
    return failures


def main():
    counts, failures = {}, []
    paths = sorted(set(CONTRACTS.glob("*.json")) | set((PROMPTS / "schema").glob("*.json")) | {PROMPTS / "versions.json"})
    documents = {path: read_json(path) for path in paths}
    counts["json_files_parsed_without_duplicate_keys"] = len(documents)
    schemas = {path: data for path, data in documents.items() if path.name.endswith(".schema.json")}
    ids = [data["$id"] for data in schemas.values()]
    require(len(ids) == len(set(ids)), "Duplicate schema $id")
    registry = Registry(retrieve=no_remote_resource).with_resources(
        (data["$id"], Resource.from_contents(data)) for data in schemas.values())
    validators, ref_count = {}, 0
    for schema in schemas.values():
        Draft202012Validator.check_schema(schema)
        resolver = registry.resolver(base_uri=schema["$id"])
        for ref in walk_refs(schema):
            resolver.lookup(ref)
            ref_count += 1
        validators[schema["$id"]] = Draft202012Validator(schema, registry=registry, format_checker=FormatChecker())
    counts["schemas_metaschema_checked"] = len(schemas)
    counts["schema_references_resolved_offline"] = ref_count
    validate_collections(documents, counts)
    cases = []
    for case in documents[CONTRACTS / "task-state-cases.json"]["schema_validation_cases"]:
        schema_id = documents[CONTRACTS / case["schema"]]["$id"]
        cases.append((case["id"], schema_id, case["input"], case["schema_valid"]))
    counts["task_schema_cases_checked"] = len(cases)
    synthetic = input_cases()
    counts["synthetic_input_schema_cases_checked"] = len(synthetic)
    counts["nonnull_report_reference_cases"] = sum("nonnull full report" in c[0] for c in synthetic)
    outcomes = Counter()
    for name, schema_id, value, expected_valid in cases + synthetic:
        errors = list(validators[schema_id].iter_errors(value))
        if schema_id in {"dsheng-task-input-v2", "dsheng-diagnosis-input-v2"}:
            errors.extend(validate_input_domains(value))
        if bool(not errors) != expected_valid:
            failures.append({"case": name, "expected_valid": expected_valid,
                "errors": [f"{e.json_path}: {e.message}" for e in errors[:3]]})
        else:
            outcomes["positive_passed" if expected_valid else "negative_passed"] += 1
    counts.update(outcomes)
    for path in validate_hashes(documents[PROMPTS / "versions.json"], counts):
        failures.append({"hash_mismatch": path})
    report = {"status": "PASS" if not failures else "FAIL", "scope": "offline_planning_contract_validation",
        "counts": counts, "failures": failures, "limits": [
            "Synthetic inputs/reports are Schema fixtures, not model responses or fabricated model history.",
            "Data/task domain cases are structurally checked, not executed or independently recalculated.",
            "Schema checks do not implement semantic support, reference ownership, state-machine or concurrency rules; this validator does check input calendar dates, closed period ordering and XLSX cell bounds.",
            "No Provider calls, business attachments, application code, database writes or network Schema retrieval.",
            "Current manifest hashes are checked; historical validation hashes are not reinterpreted as current passes."]}
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return 0 if not failures else 1


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (ValueError, KeyError, OSError) as exc:
        print(json.dumps({"status": "FAIL", "scope": "offline_planning_contract_validation",
                          "error": str(exc)}, ensure_ascii=False, indent=2))
        sys.exit(1)
