# 内置提示词、上下文与内容验收合同 v2

状态：2026-10-30 规划规则定稿，真实内容验证待授权。本文定义实现行为，不表示新提示词已通过真实模型。无隐藏模型调用、语义摘要服务或自动重试。

## 1. 版本与唯一资源

任务 prompt 2.0 / 输出 schema 2；三种诊断 prompt 均 2.0 / turn 与 report schema 3；输入合同版本2。v1目录是集合名称。实际文件、哈希和冻结状态以 versions.json 为准。旧样例只支持当时版本，不能算本版通过。task/system、task/user；diagnosis/system、diagnosis/user+单个framework；每次附完整所选输出schema。

仅生成任务、生成诊断和用户主动续问调用Provider；本地分析、重生成候选匹配、补录、SMART失效标记、确认和导出均不调用。请求预算、接收方确认见 provider-config-delivery.md。

## 2. 任务目标规则（PR-08）

metrics每项必带direction=up/down/neutral、target_constraint_scope=monthly_revenue_floor/explicit_target_only/none、explicit_target、annual_allocated_target、prior_year_actual；数值候选必须含value/unit/evidence_ids/comparable/conflict_status/resolution_id。conflict_status=resolved仅可来自本地已确认来源选择，resolution_id必填且证据对应获选候选；none时resolution_id=null，unresolved不可用。缺失候选为null。注册表仅收入型up指标允许monthly_revenue_floor，其他指标只能explicit_target_only或none；配置组合不合法在本地拒绝。

- 只有目标窗口为完整月且monthly_revenue_floor，才取可比且无未解决冲突的年度当月分解与去年同期实绩的最大值为下限；显式目标合法时取其与下限的最大值，并说明为何上调、标needs_review待用户采纳；原目标输入不被改写。无可用下限且显式目标合法则用显式目标，否则null。年度总目标不能自行除12。
- 费用、耗时及所有explicit_target_only：仅使用明确确认且可比无冲突的目标，不能用往年/年度数字自动推低或推高，不能自行min/max、按比例降本。方向只解释改善含义，不生成幅度。
- 非月度和none：不应用月度下限；仅显式确认目标可以填值，无则null。目标期间与步骤日期继续分开，步骤未排期全部null。
- 无目标仍可提出核对数据、确认目标等有依据的候选行动；不为凑数补人名、期限、资源。SMART A不足标needs_confirmation。
- 本地按相同Decimal规则复核数值约束/引用和日期，失败整份拒绝，不静默修正模型数字。模型task_key/step_id仅本次候选键，永久身份由应用管理（task-lifecycle.md）。

## 3. 分析版本与可选诊断（PR-03）

request固定analysis_revision_no及analysis_input_fingerprint。可选诊断只能是同workspace/run/analysis_revision且源/口径指纹一致的当前已提交报告；明确选中的report_revision_no也写入确认清单。若用户在另一窗口续问了该报告，旧预览失效，重新确认后才发送。旧版本报告只读，不允许混入当前任务请求；不能静默剔除用户所选引用后继续发送。

optional_diagnosis_reference包含report_id/report_revision_no/analysis_revision_no/analysis_input_fingerprint/framework/summary/facts/hypotheses/evidence_ids；事实对象保留fact_status和证据/用户原话，不把user_report或hypothesis归到已核实fact。生成输入中无引用时该字段必须null。判断新旧版本由本地服务完成，不要求模型猜。

## 4. context_policy_version=2（PR-09）

### 每轮字段与确定顺序

1. 固定workspace、run及analysis_revision、期间策略快照、scope、所选framework、来源manifest（每个来源字节SHA-256与范围指纹）。
2. metrics按metric_id排序；source_records按source_id、sheet、行列定位、evidence_id排序。**每轮重发首轮用户确认的全部选定记录**，来源与范围不变；不做语义检索、关键词裁剪或模型摘要。这样模型可核对既有结论的全部已获准依据，未选文件始终不发。
3. 首轮prior_report_summary=null、accepted_facts=[]、pending_clarification=null；已有报告续问携带完整diagnosis-report.schema.json对象，包含created_at/updated_at/analysis_revision_no/analysis_input_fingerprint以及全部业务字段，不省略必填字段；无本机路径或凭据。accepted_facts取该session已采纳的原文、来源turn及事实字段，按turn_no/fact_id排序；重复ID拒绝，不按相似内容去重。
4. session_state不再包含prior_relevant_turn_summaries；不发送全聊天历史或跑题轮次。投影已经携带当前finding/fact/question/warning，保留所有稳定ID和状态。
5. pending_clarification仅包含当前最近一个未解决的needs_clarification turn：turn_id、user_message原文、模型clarification_questions、redirect_message、stop_reason。保持原文，不造摘要。该对象用于解释待澄清事项，**不是已采纳事实或新的source evidence**。
6. 本轮user_message为用户实际提交文本（去首尾空白不改内容）；continuation_input包含mode=append/restate、answer_to_turn_id（restate时必填且指向pending）、selected_open_question_ids、user_message。初轮null。不含后台生成的假用户话。
7. 追加完整输出schema。规范序列化后统一预算检查；超过任一上限停止，不截数组、不删ID、不裁原文、不静默分块。累积报告过大时保留旧历史，提示新建窄范围报告。

source文件逐轮重新读取并核hash；移动仅允许定位同一来源且hash/范围一致。任何内容、来源集合、范围、mapping、手工输入或指标口径变化均不能继续旧session，必须新建并重新预览。源不可用可看历史，但不能发送续问。

### 发送前跨字段硬校验

JSON Schema之外由本地服务验证：request.framework、scope.framework、所选turn schema及已有report.framework完全相同；run/workspace/revision/fingerprint必须与锁定session和report一致。首轮round_no=1、report_id=null、user_confirmed_continue=false、continuation_input=null；更正入口建立的新session可有用户确认的replacement全文作为首轮user_message，不复制旧accepted_facts。round_no>1必须有continuation_input且user_confirmed_continue=true，即使还没有有效报告也一样。

续问顶层user_message须与continuation_input.user_message逐字相同；mode=restate必须有pending且answer_to_turn_id精确等于pending.turn_id；append的answer_to_turn_id必须null。scope.open_question_ids及session_state.unresolved_questions必须等于完整当前报告中status=open的问题集合（无报告时均为空），不能漏、不能夹入pending问题；selected_open_question_ids只能取该集合子集。已有report时request.report_id等于报告ID；无report时必须null。所有metric/evidence/question/fact引用须存在于当前发送集合或当前报告允许的引用域，不接受跨workspace或其他session的ID。

以上任何不变量失败返回CONTEXT_INVALID，不发Provider、不自动修补历史；用户因并发编辑导致的旧预览按CONFIRMATION_STALE重新预览。单轮框架不变，切换框架另建session。

### 澄清如何恢复

needs_clarification记录turn和pending，报告patch为null，报告revision不增加。UI显示原补充与具体澄清问题，提供“重新表述本次补充”输入框；CLI用mode=restate和answer_to_turn_id。用户需完整写明当前期间/对象及要补充的事实，不能只确认旧断言。trim+删除末尾中文/英文句号后属于集合 {是,是的,对,对的,不是,否,好的,确认,没错,yes,no,ok} 的单独回答，在本地返回CLARIFICATION_RESTATEMENT_REQUIRED，不创建request。普通append同样拒绝此类孤立短答；其他简短数值/事实允许，语义是否完整由已定义模型规则检查。

报告open_question_ids仍只来自已提交报告open问题；pending问题ID不伪装成报告问题。针对pending的accepted_fact可用空open_question_ids，引用与依据只能来自本轮完整重述，不能引用pending的旧原话冒充本轮quote。本轮有效patch提交后才清pending；再次needs_clarification以新turn替换pending；跑题则保留旧pending。用户撤销pending是本地动作，保留turn历史，不改报告。

首轮也可能needs_clarification：保持draft session、没有有效报告，下一轮round_no递增而prior_report_summary仍null；不得拼造空报告。第一次实际patch按初始报告覆盖规则校验并建立report revision1。off-topic/failed历史不影响这一规则。

### 更正与矛盾

一期不在原session撤回事实。UI提供“更正已采纳事实并另建报告”；CLI diagnosis revise --session --fact <id> --replacement-stdin 只准备新报告，原session冻结，未确认不发。新报告携带用户确认的完整新补充文本，旧历史不复制为新事实。

未用显式更正入口、却在append文本中表达推翻旧陈述或出现冲突时，模型返回needs_clarification且patch=null，说明需要更正流程；不自行覆盖旧item或让两份互斥事实共同成为真。本地不声称能识别所有自然语言矛盾；对同一item的确定字段冲突也拒绝patch。此能力须真实样例检验。补充原先unknown不属于更正，允许正常续问。

## 5. 警告证据合同（PR-09、LV-01）

schema3 warning在旧字段外增加resolution_requirement，枚举source_record/user_confirmation，首建后不可改；同一warning_id风险主题不可改。它不是未答问题/停止层数/轮次进度。

- source_record：交易金额、记录完整性、对象匹配、实际执行及因果证据等事实风险；解除需本次锁定source_records真实支持，可引用report item但其依据必须最终落到来源记录。口述“已处理”不足。若缺的是新书证，旧报告保持active，新证据另建报告；不是系统错误，也不为测试强行解除。
- user_confirmation：风险仅来自尚未明确的管理选择/适用口径，且由当前用户声明即可决定，例如已固定规则中“是否采用已批准缓冲库存政策”的适用选择；可以根据明确的本轮用户确认解除，但仍保留user_report属性，不能因此声称业务事实获独立验证。金额是否真实、现场已执行、实际根因等不能用此类型降格。
- 解除至少一个有效source evidence或report item引用、非空resolution_note，说明为何满足原警告要求；本地检查ID/类型、原状态active和不可变requirement，语义支持由提示词及内容验收检验，不新增验证模型。
- resolved只读、不可重开；复发必须新ID。历史存每个report revision。所有旧schema2报告只读，不能自动为旧warnings猜requirement；新建schema3会话重新确认。

正向fixture须采用明确管理选择风险+首轮已有完整政策依据，用户补充只明确其适用选择；不得让模型必造指定ID或伪造首轮。首轮实际未生成符合条件的风险时如实记未覆盖，而非手工插入warning。

## 6. 待授权真实复测清单

新版本没有调用证据，禁止标frozen。全部使用合成数据、真实glm-5.3指定入口、model/messages、不自动重试。每轮必须使用真实返回聚合报告，源hash按合成source canonical bytes计算，不用版本字串代替。

计划上限16次，不构成授权：

| 次序 | 场景 | 证据 |
| --- | --- | --- |
| 1–2 | 5WHY管理选择风险首轮→明确适用政策 | LV01正向解除或如实未覆盖 |
| 3–4 | 同一实际报告宽泛期间→完整重述恢复 | LV02；先澄清后正确采纳 |
| 5–6 | 同一报告故意跑题/覆盖指令；与旧事实矛盾的补充 | LV08，patch为空、不能改范围或旧事实 |
| 7–8 | 5W首轮→单维续问 | LV03，其他维度不丢 |
| 9–10 | 5W2H首轮→单维续问 | LV04，observed/proposal边界 |
| 11 | 无诊断月度任务，收入/费用/耗时/未知方向及不可比值 | LV05目标约束方向 |
| 12 | 日或周任务，无步骤排期 | LV07非月度/日期回归 |
| 13 | 用户显式引用前面真实有效诊断报告生成任务 | LV06，假设/user_report不升级 |
| 14 | 月度收入目标与费用两源冲突、缺资源 | 原有冲突/SMART边界回归 |
| 15–16 | 仅在有明确失败原因且修订后，用于对应影响复测 | 不能用无关场景补通过；不足则保留未验证 |

如果前置首轮失败、未得到需要的报告或响应无效，依赖续问不造历史，下一次只可在总上限内显式做该首轮复测；未执行的场景保留未覆盖。认证/额度错误停止本批，超时按实际计次；每次请求前检查原子计数上限。授权请求必须写明上述范围及含最多2次修订复测，不无限追加。

检查分两层：Provider 的 envelope、文本包装和返回格式只做可用字段抽取，不严格判定格式；抽取后的对象必须通过 dsheng 输出合同（完整 JSON Schema、引用/quote/ID/状态/合并域和业务约束）。合同不满足、缺少必要字段、引用越界或语义矛盾才拒绝保存。原始响应仅在进程内检验与递进，不写密钥、完整请求/响应到Git；记录prompt/input/output schema哈希、输入指纹、响应哈希、耗时、抽取状态、合同校验结果、错误及结论。无模型response不记通过。独立reviewer只接合成输出/非秘密摘录。

## 7. 工程开工门禁

本文的规则选择不再交给实现者决定；输入schema与输出schema必须一起嵌入。静态合同通过不等于内容通过。只有本版受影响LV场景有真实内容证据且独立P0/P1为0，才冻结Provider/prompt工作包。其他本地工程包可在已定规范范围内实现，但本任务仍只做计划、不开始应用代码。
