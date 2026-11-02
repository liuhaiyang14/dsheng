#!/usr/bin/env python3
"""Offline provider-output extraction check for the dsheng prompt contracts.

Reads a saved provider response body (a text file or stdin) and reports which
decodable top-level JSON object, if any, satisfies a chosen output-contract
schema. This preserves the acceptance policy agreed with the user: provider
envelope/text wrapper differences are adapted during extraction, while the
extracted object must still pass the full output contract.

This tool is strictly offline: it never opens a network connection, never
reads credentials, and only sees whatever response text it is given. It exists
because the 2026-11-02 retest showed that picking the *largest* JSON object in
a response can grab a SMART-criteria block instead of the contract root; the
selection rule here is contract-first.

Usage:
  python3 tools/extract_provider_json.py --schema crates/dsheng-core/prompts/v1/schema/task-output.schema.json response.txt
  python3 tools/extract_provider_json.py --selftest
"""

import argparse
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ROOT_KEYS = {"schema_version", "summary", "tasks"}


def candidates(text):
    """Decode every top-level JSON object that starts at a '{' in the text."""
    found = []
    for match in re.finditer(r"\{", text):
        try:
            obj, end = json.JSONDecoder().raw_decode(text[match.start():])
        except Exception:
            continue
        found.append((match.start(), end, obj))
    return found


def task_subvalidator(root_schema, registry):
    """Validator for one element of tasks[] (the $defs/task subschema)."""
    from jsonschema import Draft202012Validator, FormatChecker
    sub = {"$defs": root_schema.get("$defs", {}), "$ref": "#/$defs/task"}
    return Draft202012Validator(sub, registry=registry, format_checker=FormatChecker())


def wrap_bare_task(task):
    """Option-C fallback: rebuild the Schema 2 root envelope around one bare task.

    Only mechanical, client-owned envelope fields are reconstructed; no business
    value is invented. summary falls back to the task's own summary string
    (model-generated content) or an empty string. The result must carry
    envelope_locally_reconstructed=True wherever it is recorded.
    """
    summary = task.get("summary") if isinstance(task.get("summary"), str) else ""
    return {"schema_version": "2", "summary": summary, "tasks": [task]}


def choose(text, validator, task_validator=None):
    """Return (candidate, stage, candidate_keys) following the contract-first rule.

    Stages:
      contract_matched       - a candidate passes the schema; use it.
      root_shape_but_invalid - closest candidate carries root keys but fails the schema.
      wrapped_bare_task      - adapt policy only: exactly one candidate validates as a
                               bare task; the root envelope was rebuilt locally and the
                               caller must record envelope_locally_reconstructed=True.
      ambiguous_bare_tasks   - adapt policy only: several bare-task candidates; refuse.
      no_contract_candidate  - no candidate carries root keys; report the largest.
      no_json_object         - nothing decodable.
    """
    cands = candidates(text)
    if not cands:
        return None, "no_json_object", []
    keys = [sorted(c.keys()) if isinstance(c, dict) else type(c).__name__ for _, _, c in cands]

    def safe_errors(obj, v):
        # An unresolvable $ref raises instead of yielding an error; degrade to a
        # reported failure rather than crashing the whole extraction.
        try:
            return list(v.iter_errors(obj))
        except Exception as exc:
            return [f"schema-resolution-error: {type(exc).__name__}: {exc}"]

    for _, _, obj in cands:
        if validator is not None and not safe_errors(obj, validator):
            return obj, "contract_matched", keys
    # "summary" alone is NOT a root marker: task objects carry their own summary
    # field, so only schema_version/tasks count as strong root-shape evidence.
    strong = [obj for _, _, obj in cands if isinstance(obj, dict) and ({"schema_version", "tasks"} & set(obj.keys()))]
    if strong:
        best = max(strong, key=lambda c: len(ROOT_KEYS & set(c.keys())))
        return best, "root_shape_but_invalid", keys
    if task_validator is not None:
        bare = [obj for _, _, obj in cands if isinstance(obj, dict) and not safe_errors(obj, task_validator)]
        if len(bare) == 1:
            return wrap_bare_task(bare[0]), "wrapped_bare_task", keys
        if len(bare) > 1:
            return max(bare, key=lambda c: len(json.dumps(c, default=str))), "ambiguous_bare_tasks", keys
    largest = max((obj for _, _, obj in cands), key=lambda c: len(json.dumps(c, default=str)))
    return largest, "no_contract_candidate", keys


def build_validator(schema_path):
    try:
        from jsonschema import Draft202012Validator, FormatChecker
        from referencing import Registry, Resource
    except ImportError:
        print("jsonschema/referencing not installed; running key-level checks only", file=sys.stderr)
        return None
    schemas = []
    for directory in (ROOT / "contracts" / "v1", ROOT / "crates" / "dsheng-core" / "prompts" / "v1" / "schema"):
        for path in sorted(directory.glob("*.schema.json")):
            schemas.append(json.loads(path.read_text()))
    registry = Registry(retrieve=lambda uri: (_ for _ in ()).throw(Exception("remote schema blocked")))
    registry = registry.with_resources((s["$id"], Resource.from_contents(s)) for s in schemas if "$id" in s)
    schema = json.loads(Path(schema_path).read_text())
    return Draft202012Validator(schema, registry=registry, format_checker=FormatChecker())


def selftest():
    root = '{"schema_version":"2","summary":"ok","tasks":[]}'
    smart = json.dumps({"S": "x" * 200, "M": "y" * 200, "A": "z" * 200, "R": "w" * 200, "T": "v" * 200})
    bare = '{"task_key":"k","title":"t","steps":[]}'
    text = f"分析：{smart}\n最终：{root}"
    obj, stage, keys = choose(text, None)
    assert stage == "root_shape_but_invalid" or obj is not None
    # contract-first: root-shaped candidate must win over the larger SMART block
    assert isinstance(obj, dict) and "schema_version" in obj, obj
    obj, stage, _ = choose(f"结果：{bare}", None)
    assert stage == "no_contract_candidate" and obj == json.loads(bare)
    obj, stage, _ = choose("no json here", None)
    assert obj is None and stage == "no_json_object"
    # option-C fallback: a fake task_validator that accepts any task_key-bearing dict
    class AnyTask:
        def iter_errors(self, o):
            return [] if isinstance(o, dict) and "task_key" in o else [AssertionError("no")]
    obj, stage, _ = choose(f"结果：{bare}", None, task_validator=AnyTask())
    assert stage == "wrapped_bare_task", stage
    assert obj["schema_version"] == "2" and obj["tasks"] == [json.loads(bare)], obj
    two = json.dumps(json.loads(bare)) + " " + json.dumps({"task_key": "k2", "title": "t2", "steps": []})
    obj, stage, _ = choose(two, None, task_validator=AnyTask())
    assert stage == "ambiguous_bare_tasks", stage
    print("selftest passed")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("response", nargs="?", help="saved provider response text file; omit to read stdin")
    ap.add_argument("--schema", help="output-contract schema to validate the chosen candidate against")
    ap.add_argument("--policy", choices=["strict", "adapt"], default="strict",
                    help="strict: extracted object must already satisfy the contract; "
                         "adapt: additionally wrap one sole bare-task candidate into the "
                         "Schema 2 root envelope and flag envelope_locally_reconstructed")
    ap.add_argument("--selftest", action="store_true", help="run built-in checks and exit")
    args = ap.parse_args()
    if args.selftest:
        selftest()
        return
    text = Path(args.response).read_text() if args.response else sys.stdin.read()
    validator = build_validator(args.schema) if args.schema else None
    task_validator = None
    if args.policy == "adapt" and validator is not None:
        try:
            from referencing import Registry, Resource
            root_schema = json.loads(Path(args.schema).read_text())
            schemas = []
            for directory in (ROOT / "contracts" / "v1", ROOT / "crates" / "dsheng-core" / "prompts" / "v1" / "schema"):
                for path in sorted(directory.glob("*.schema.json")):
                    schemas.append(json.loads(path.read_text()))
            reg = Registry(retrieve=lambda uri: (_ for _ in ()).throw(Exception("remote schema blocked")))
            reg = reg.with_resources((s["$id"], Resource.from_contents(s)) for s in schemas if "$id" in s)
            task_validator = task_subvalidator(root_schema, reg)
        except Exception as exc:
            print(f"task subvalidator unavailable: {type(exc).__name__}: {exc}", file=sys.stderr)
    obj, stage, keys = choose(text, validator, task_validator)
    report = {"policy": args.policy, "stage": stage, "candidate_top_level_keys": keys[:12]}
    if stage == "wrapped_bare_task":
        report["envelope_locally_reconstructed"] = True
    if obj is None:
        report["result"] = "unextractable"
    elif validator is None:
        report["result"] = "extracted_no_validator"
    else:
        try:
            errors = [f"{e.json_path}: {e.message}" for e in validator.iter_errors(obj)]
        except Exception as exc:
            errors = [f"schema-resolution-error: {type(exc).__name__}: {exc}"]
        report["result"] = "contract_pass" if not errors else "contract_fail"
        report["contract_errors"] = errors[:10]
    print(json.dumps(report, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
