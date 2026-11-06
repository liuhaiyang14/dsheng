-- Report series metadata preserves immutable report IDs, bodies and existing references.
-- Match Rust Unicode White_Space at the boundaries only; preserve interior whitespace.
-- Keep this trim character set identical to diagnosis::report_head.
ALTER TABLE diagnosis_reports ADD COLUMN series_id TEXT NOT NULL DEFAULT '';
ALTER TABLE diagnosis_reports ADD COLUMN series_version_no INTEGER NOT NULL DEFAULT 1 CHECK (series_version_no >= 1);

WITH ranked AS (
    SELECT report_id,
           FIRST_VALUE(report_id) OVER (
             PARTITION BY workspace_id, run_id, framework, trim(problem_statement, char(9,10,11,12,13,32,133,160,5760,8192,8193,8194,8195,8196,8197,8198,8199,8200,8201,8202,8232,8233,8239,8287,12288))
             ORDER BY created_at, report_id
           ) AS series,
           ROW_NUMBER() OVER (
             PARTITION BY workspace_id, run_id, framework, trim(problem_statement, char(9,10,11,12,13,32,133,160,5760,8192,8193,8194,8195,8196,8197,8198,8199,8200,8201,8202,8232,8233,8239,8287,12288))
             ORDER BY created_at, report_id
           ) AS version
    FROM diagnosis_reports
)
UPDATE diagnosis_reports
SET series_id = (SELECT series FROM ranked WHERE ranked.report_id = diagnosis_reports.report_id),
    series_version_no = (SELECT version FROM ranked WHERE ranked.report_id = diagnosis_reports.report_id);

CREATE UNIQUE INDEX diagnosis_report_series_version ON diagnosis_reports(series_id, series_version_no)
WHERE series_id <> '';
CREATE INDEX diagnosis_report_series_scope ON diagnosis_reports(workspace_id, run_id, framework, trim(problem_statement, char(9,10,11,12,13,32,133,160,5760,8192,8193,8194,8195,8196,8197,8198,8199,8200,8201,8202,8232,8233,8239,8287,12288)));

-- Keep direct import/test inserts and normal saves on the same atomic numbering path.
CREATE TRIGGER diagnosis_report_assign_series AFTER INSERT ON diagnosis_reports
WHEN NEW.series_id = ''
BEGIN
  UPDATE diagnosis_reports
  SET series_id = COALESCE((
        SELECT r.series_id FROM diagnosis_reports r
        WHERE r.report_id <> NEW.report_id AND r.workspace_id = NEW.workspace_id
          AND r.run_id = NEW.run_id AND r.framework = NEW.framework
          AND trim(r.problem_statement, char(9,10,11,12,13,32,133,160,5760,8192,8193,8194,8195,8196,8197,8198,8199,8200,8201,8202,8232,8233,8239,8287,12288)) = trim(NEW.problem_statement, char(9,10,11,12,13,32,133,160,5760,8192,8193,8194,8195,8196,8197,8198,8199,8200,8201,8202,8232,8233,8239,8287,12288)) AND r.series_id <> ''
        ORDER BY r.series_version_no DESC LIMIT 1
      ), NEW.report_id),
      series_version_no = COALESCE((
        SELECT MAX(r.series_version_no) FROM diagnosis_reports r
        WHERE r.report_id <> NEW.report_id AND r.workspace_id = NEW.workspace_id
          AND r.run_id = NEW.run_id AND r.framework = NEW.framework
          AND trim(r.problem_statement, char(9,10,11,12,13,32,133,160,5760,8192,8193,8194,8195,8196,8197,8198,8199,8200,8201,8202,8232,8233,8239,8287,12288)) = trim(NEW.problem_statement, char(9,10,11,12,13,32,133,160,5760,8192,8193,8194,8195,8196,8197,8198,8199,8200,8201,8202,8232,8233,8239,8287,12288))
      ), 0) + 1
  WHERE report_id = NEW.report_id;
END;
