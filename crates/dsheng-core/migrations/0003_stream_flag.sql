-- dsheng SQLite schema 0003：providers.stream 放宽（流式输出生成链路）。
--
-- 合同：docs/specs/provider-config-delivery.md §1/§3（2026-11-04 用户批准增量）：
--   * stream 默认 false，可选 true（请求带 "stream": true）；
--   * 流式下 response_seconds = 相邻数据块空闲上限（chunk 间空闲语义）；
--   * total_seconds 上限扩至 1800（流式生成可能较长）。
--
-- 0001 现状：providers.stream 带 `CHECK (stream = 0)`（当时增量未批准），
-- 且 total_seconds 上限仍为 900（旧值）。0003 按标准做法重建 providers 表
-- （0001 冻结文件不改；其余列/约束/唯一键/外键原样重声明）：
--   * stream 放宽为 `CHECK (stream IN (0,1))`（仍 NOT NULL DEFAULT 0）；
--   * total_seconds 放宽为 `BETWEEN 60 AND 1800`（settings.schema 已同步）；
--   * 既有行回填 stream=0（0001 CHECK 已保证存量恒 0；COALESCE 仅为防御）。
--
-- 无任何表 REFERENCES providers（0001 中引用列均无外键），重建安全；
-- 索引：providers 无二级索引，重建后无需重建。

CREATE TABLE providers_0003 (
    provider_id              TEXT PRIMARY KEY,
    workspace_id             TEXT NOT NULL REFERENCES workspaces(workspace_id),
    revision                 INTEGER NOT NULL DEFAULT 1 CHECK (revision >= 1),
    name                     TEXT NOT NULL,
    protocol                 TEXT NOT NULL DEFAULT 'chat_completions' CHECK (protocol = 'chat_completions'),
    base_url                 TEXT NOT NULL,
    model                    TEXT NOT NULL,
    stream                   INTEGER NOT NULL DEFAULT 0 CHECK (stream IN (0, 1)),
    -- 明文 Key：已确认产品选择；仅服务层内存使用，任何查询不得投影本列
    api_key                  TEXT,
    connect_seconds          INTEGER NOT NULL DEFAULT 15 CHECK (connect_seconds BETWEEN 1 AND 60),
    response_seconds         INTEGER NOT NULL DEFAULT 180 CHECK (response_seconds BETWEEN 30 AND 600),
    total_seconds            INTEGER NOT NULL DEFAULT 240 CHECK (total_seconds BETWEEN 60 AND 1800),
    context_window_tokens    INTEGER NOT NULL DEFAULT 32768,
    output_reserve_tokens    INTEGER NOT NULL DEFAULT 8192,
    created_at               TEXT NOT NULL,
    updated_at               TEXT NOT NULL,
    UNIQUE (workspace_id, name),
    CHECK (total_seconds >= connect_seconds + response_seconds),
    CHECK (output_reserve_tokens * 2 <= context_window_tokens)
);

INSERT INTO providers_0003
    (provider_id, workspace_id, revision, name, protocol, base_url, model, stream,
     api_key, connect_seconds, response_seconds, total_seconds, context_window_tokens,
     output_reserve_tokens, created_at, updated_at)
SELECT provider_id, workspace_id, revision, name, protocol, base_url, model,
       COALESCE(stream, 0), api_key, connect_seconds, response_seconds, total_seconds,
       context_window_tokens, output_reserve_tokens, created_at, updated_at
  FROM providers;

DROP TABLE providers;
ALTER TABLE providers_0003 RENAME TO providers;
