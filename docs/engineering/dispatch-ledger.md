# 真实模型调用台账（工程期，主 Agent 逐次记录）

合同：真实请求仅用户逐批授权；每次 ≤1 HTTP 请求；不自动重试；
失败/超时如实记录；密钥不回显不入库。规划期 44 次派发见 versions.json（历史事实）。

## 批次 2026-11-04-A（用户授权 4 次）

- 授权范围：5WHY / 5W / 5W2H 三框架诊断首轮各 1 次＋任务单生成 1 次
- Provider：ZAI `glm-4.7`（api.z.ai/api/coding/paas/v4；Key 经 stdin 静默配置）
- 数据：用户真实两表（~/Desktop/ds，核算表＋推移表；临时库 /tmp，不入 git）
- 确认流程：每次 fresh `--dry-run` → `--confirmation-id --confirm-send`

| # | 操作 | confirmation_id | 请求字节/保守tokens | 结果 | 耗时 | 产物 |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | diagnosis generate 5why | （派发时填） | | | | |
| 2 | diagnosis generate 5w | | | | | |
| 3 | diagnosis generate 5w2h | | | | | |
| 4 | task generate | | | | | |
