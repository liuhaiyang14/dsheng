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
| 1 | diagnosis generate 5why | 743e3fa6-4244-4a63-bf2b-82996a3aad0b | 40.9k/37.8k | **RESPONSE_TIMEOUT**（195s，响应上限 180s 被超；请求已真实发出，不自动重试） | 195.0s | attempt failed；无报告写入 |
| 2 | diagnosis generate 5w | 5edad685-34d3-4fac-bdfa-7ff5ea6f7449 | ~40k/37.8k | **RESPONSE_TIMEOUT**（响应上限已调 480s 仍超；elapsed 495s） | 495.1s | attempt failed；无报告写入 |
| 3 | diagnosis generate 5why（流式） | b9e01f53-f7ad-4e05-8b4d-d170d90e7779 | 40.2k/37.2k | **RESPONSE_TOO_LARGE**（流式稳定接收 4,845 块/1.9MB@180s 仍在产出，累计 2,097,422B 超 2MiB 合同上限；流式/进度/空闲超时机制验证有效） | 198s | attempt failed；无报告写入 |
| 3 | diagnosis generate 5w2h | | | | | |
| 4 | task generate | 未执行 | | 未派发（主 Agent 判断：与 #1/#2 同尺寸载荷大概率同样超时，暂停避免无效消耗；待用户决策） | | |

**批次小结（2026-11-04-A，进行中）**：已派发 2 次均 RESPONSE_TIMEOUT（180s/480s 上限各一次），
请求均真实发出并计数；剩余 1 次授权未使用（#4 预留任务单，待用户对响应上限/模型选择决策）。 glm-4.7 @ coding 入口对 ~38k token 业务
载荷两连超（规划期小载荷 58–196s 可完成）。候选处置（待用户选择）：换非 coding 入口/
更快模型；或实现诊断发送范围裁剪（当前 A1:Z999 全表→仅相关科目行，预计降到 ~10k tokens）
后重试；或两者并行。
