-- 2026-11-06 用户授权的月度周期起点；旧策略保持每月1日，不改写历史快照。
ALTER TABLE period_policies
ADD COLUMN month_start_day INTEGER NOT NULL DEFAULT 1
CHECK (month_start_day BETWEEN 1 AND 31);
