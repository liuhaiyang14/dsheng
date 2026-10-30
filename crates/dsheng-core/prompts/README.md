# Built-in Prompt Specifications

`v1/` contains candidate prompt and JSON Schema resources for the desktop app and `dsheng` CLI. They contain no real business data, Provider credentials, or customer examples. Version identifiers and validation status are in `v1/versions.json`.

## Message Assembly

- task_skill_context contains rule-version and structural metadata only; it does not carry rule prose or override the built-in system message.
- Task generation: `v1/task/system.md` is the system message. Render `v1/task/user.md` with `{{payload_json}}` and the complete parsed `v1/schema/task-output.schema.json` in `{{output_schema_json}}`.
- Business diagnosis: concatenate `v1/diagnosis/system.md` and exactly one selected framework file as the system message. Render `v1/diagnosis/user.md` with `{{payload_json}}` and the complete parsed schema selected by framework in `{{output_schema_json}}`.
- The only framework resources are `framework-5why.md`, `framework-5w.md`, and `framework-5w2h.md`. Do not include unselected framework instructions in the request.
- Diagnosis schema selection is exact: `diagnosis-turn-5why.schema.json`, `diagnosis-turn-5w.schema.json`, or `diagnosis-turn-5w2h.schema.json`. Never include all three schemas in one request.

The app embeds these resources in its build. They are not editable through the UI, CLI options, workspace files, or Provider settings. Runtime data belongs in the structured user payload, never in these files.

## Schema Resources

- Task response: `v1/schema/task-output.schema.json`.
- Diagnosis turns: `v1/schema/diagnosis-turn-5why.schema.json`, `v1/schema/diagnosis-turn-5w.schema.json`, or `v1/schema/diagnosis-turn-5w2h.schema.json`, selected by the locked session framework.
- Persisted diagnosis report and `diagnosis show --json`: `v1/schema/diagnosis-report.schema.json`.
- Prompt/schema versions and validation/freeze status: `v1/versions.json`.
- The app loads the selected JSON Schema as a structured object and inserts its complete contents in `{{output_schema_json}}`. The file path or schema ID alone is not sent in place of its contents.
- Schema validation is followed by service checks that JSON Schema cannot express: evidence IDs belong to the current payload, user quotes are exact substrings, findings belong to the selected framework and report, and initial 5W/5W2H dimension coverage or 5WHY chain continuity is valid.

## Validation Status

Overall plan readiness review on 2026-10-30 found additional unresolved contracts, including metric direction and multi-turn context policy. See [the plan review](../../../docs/plan-readiness-review-2026-10-30.md). Engineering readiness is false; positive warning resolution is not the only remaining gate. The sample results below remain historical evidence for their exact tested versions, not overall approval.

Current resources are candidate, partial_validated_pending_positive_warning_resolution. Task prompt is 1.3/schema 1; diagnosis prompts are 5WHY 1.3 and 5W/5W2H 1.4, with turn/report schema 2. The v1 directory is a resource collection name, not a shared output schema version.

The 2026-10-30 real Z.ai Coding Plan tests used glm-5.3 and synthetic business inputs only. Current task resources passed daily and monthly content checks, including comparable monthly floors, unresolved source conflicts, missing daily targets, and null step dates without scheduling evidence. Current diagnosis resources passed the three initial frameworks, genuine relevant continuation with the full actual prior report, refusal to resolve unsupported risks, scope clarification and deliberate off-topic redirection.

The final mapping-confirmation pair kept the warning active because oral confirmation did not supply written mapping evidence. This was appropriately conservative, but did not exercise positive active-to-resolved behavior. That acceptance remains unverified; the prompt/Provider work package is not frozen. The main planning agent owns this gap; implementation agents must not invent the methodology or quietly weaken the gate.

See VALIDATION.md for the scenario matrix, failure history, limitations and exact remaining gate. v1/versions.json records every observed batch, versions, prompt/schema hashes, response hashes and request counts. All 8+5+2 newly authorized requests have been used; remaining allowance is zero. Historical failures and up to five earlier unobserved dispatches are retained, not credited as successes.

No raw business file, credential, full request or full response artifact was stored in this repository. Actual prior report state stayed in process memory for continuation. Tests do not establish product persistence, UI/CLI E2E, file-hash integrity or statistical reliability. The mapping fixture used a version-derived synthetic hash, explicitly not a source-byte integrity check.

## Change Rules

The product behavior and prompt contracts are defined by the implementation plan. Engineering agents implement these resources and their assembly; they do not independently research or redesign the business methods. These versions remain candidates until the remaining semantic acceptance and live checks recorded in `v1/versions.json` pass. A behavior change requires the main agent to update the plan, prompt text, JSON Schema, version metadata, and acceptance criteria together, then rerun the affected real-Provider checks.

Every prompt change must be reviewed for evidence attribution, scope isolation, prompt injection, off-topic handling, schema compatibility, and token/context impact. Bump `prompt_version` when prompt behavior or text changes materially. Bump `schema_version` when input or output fields change.
