# 任务、分析版本与本地数据生命周期合同 v1

状态：2026-10-30 规划补齐的确定性执行规则。本文关闭 PR-03、PR-04、PR-05、PR-07 的规则选择；实现及产品测试尚未执行。与旧计划中“按模型 task_key 自动更新”“未生成旧项自动替代”“单个 cycle_role 属于任务自身”的概述冲突时，以本合同为准。本文不修改模型输出 Schema，不要求模型识别旧任务，也不增加模型调用。

编辑输入：[task-edit.schema.json](../../contracts/v1/task-edit.schema.json)。候选合并输入：[task-reconcile.schema.json](../../contracts/v1/task-reconcile.schema.json)。合成领域验收：[task-state-cases.json](../../contracts/v1/task-state-cases.json)。Schema 验证使用 Draft 2020-12 并启用 format 校验；跨对象、日期顺序与业务状态仍由本合同指定的服务校验完成。

## 1. 永久身份与作用域

- task_id、step_id、event_id、mutation_id、candidate_generation_id 由应用生成 UUID。任务永久身份属于 workspace，不能以 run、标题、指标或模型返回字符串作为永久身份。
- 模型 task_key 只在一次 candidate_generation 内唯一；模型 step_id 只在对应候选任务内唯一；step.order 必须为 1..N 且无重复。重复键或不连续顺序作为领域验证失败拒绝整份候选，不重命名掩盖错误。task_key 和 model step_id 仅存于不可变生成快照与候选映射，不充当数据库主键。
- 步骤永久归属于一个任务，不允许移到另一任务；任务不得跨 workspace 移动。另一企业的任何 ID 均返回 WORKSPACE_MISMATCH，不因 UUID 已知而绕过隔离。
- 任务当前字段由 tasks/task_steps 保存，内容与进度共同使用单调 task_revision_no；每个成功事务只加 1，一组步骤修改不逐行加多次。任务创建初值 1。所有写入要求 expected_task_revision，冲突返回 REVISION_CONFLICT，不自动重放旧 patch。
- 任务执行状态与记录生命周期分开：status 是 not_started/in_progress/completed/blocked；lifecycle 是 active/superseded。被显式替代的任务只读、可检索，旧进度与事件仍在；不能把 superseded 当“已完成”。

## 2. 分析版本、输入指纹与提交门禁

### 2.1 不可变分析 revision

每次 analyze 成功都会事务创建一份不可变 analysis revision，包括第一次 revision 1；analysis_runs.current_revision_no 是当前指针。不能只保存旧 revision 而漏掉当前输入快照。相同 workspace+period_key 重做仍保留 run_id，但增加 revision_no；输入相同也产生新的显式重做记录，不自动把旧诊断当作新版结果。

analysis revision 固定：来源内容 SHA-256/工作表/范围、映射版本和映射值、指标定义和版本、用户手工输入、已确认来源冲突选择、周期策略/时区/业务范围、本期和目标期、规范化指标、比较值与其 base_run_id/base_revision_no。若比较由源内可比列取得，记录 source evidence，不伪造 base_run_id；两个 ID 为 null。跨多个不重叠基期 revision 聚合时记录完整 base_refs 数组，不只保留最后一个基期。

analysis_input_fingerprint 是上述结构的 SHA-256。canonicalization_version 固定 dsheng-canonical-json-v1：对象键按 Unicode 码点升序；数组按各合同固定顺序；来源按 source_id/sheet/range，指标按 metric_key/scope，比较按 type/base_ref；数值统一为无多余前导零/末尾零的十进制字符串，-0 为 0；日期 ISO date，时间 UTC ISO-8601；UTF-8，无无意义空白，不做会改变业务原文的 Unicode 归一化。缺字段与 null 不等价，字段齐全后再 hash。不包含绝对路径、API Key 或可恢复凭据的派生值。

### 2.2 每次生成绑定

所有 generation session/attempt、diagnosis report、candidate output 保存：workspace_id、run_id、analysis_revision_no、analysis_input_fingerprint。另生成 generation_input_fingerprint，覆盖 workflow、问题/框架或目标期、用户选择的 source_manifest、字段/数据范围、运行时用户补充与上下文 manifest、可选诊断引用的 report_id/report_revision_no、所用 prompt/schema/context-policy 版本。Provider 路由确认另按配置合同绑定 provider_id/provider_revision，不能把 Key 混入业务指纹。

诊断引用的准入同时要求：同 workspace、同 run_id、同 analysis_revision_no、同 analysis_input_fingerprint、报告可读且所选 report_revision_no 仍与发送预览一致。用户可选择最新报告或取消引用，但不能只凭 run_id 相同把旧版报告当当前事实。无报告/未选择报告不妨碍任务生成；**已选择但失效**则本地返回 STALE_DIAGNOSIS_REFERENCE，提示重选或取消后重新确认，不静默移除/更换。

源路径重定位仍遵守来源身份、范围和内容 hash 不变的旧合同。手工值、映射、目标期或公式变化即使源文件 hash 不变，也会使分析 revision/指纹改变。旧 session 冻结为 frozen_analysis_revision；旧报告保留只读，可明确查看其所属 revision，不可继续为新分析写结果。

在 preparing 前及最终提交事务内检查：analysis_runs.current_revision_no、指纹、目标 scope、引用 report revision、候选/session revision 与预览一致。任意变化返回 INPUT_REVISION_CHANGED，attempt 结束 failed，保留元数据和上一有效结果，未提交的模型正文不落库。Provider configuration revision 与 confirmation 只在派发前按配置合同校验；已经派发的请求使用固定路由/模型快照，之后更换 Key 或其他 Provider 设置不追改已发送路由，也不单独导致有效返回提交失败，结果保留实际使用的 provider_revision。新数据分析可进行，无需因等模型而锁住整个数据库；通过乐观校验避免旧结果污染新版本。

比较结果读取冻结的 base revision；日后基期重做不得改旧分析的同比/环比。重新 analyze 才选择当时最新兼容基期并产生新 revision。

## 3. 初次生成与重生成的应用行为

### 3.1 初次生成

首次 task generate 成功且本 run 当前分析 revision 从未应用过任务生成、同时没有从历史或跨期关联来的候选目标期任务时，可直接事务创建永久任务及步骤，创建相应 period links；用户已明确点击生成，不增加第二次模型或提交确认。任何新步骤 status 一律 not_started、completed_at=null；模型 status 仅保留在生成快照，不把“模型写 completed”视为执行证据。

只要当前 run/目标期已有任何 active 任务关联、已有 applied candidate，或本次为显式重生成，返回 review_required 并只存候选，不改 tasks/task_steps。空候选 tasks=[] 也是有效候选；不能把它解释为删除全部旧任务。

### 3.2 本地差异确认

UI 显示“生成建议”和现有任务两栏；不按标题近似、相同指标或相同模型 key 自动匹配。用户逐项选择：

| 操作 | 永久身份 | 结果 |
| --- | --- | --- |
| new | 应用创建 task/step UUID | 使用候选内容创建，新步骤未开始；至少一条有效步骤。 |
| update | 用户指定现有 task/step UUID | 仅 take_fields 明确列出的内容用候选值覆盖；空列表表示保留内容。其余字段/进度不变。 |
| replace | 用户指定旧 task UUID 并填写原因 | 新任务使用新 UUID；旧任务 lifecycle=superseded，只读但历史保留。旧事件和进度不复制成新任务进展。 |
| ignore | 无新身份 | 忽略这项候选，仍保存用户选择和候选快照。 |

new/replace 会为新任务创建本次 run/revision 的 plan link，source_period/target_period 来自已校验候选且须等于本次请求期间。update 为被选现有任务创建或显式更新同一 run/revision 的 plan link，预览展示该 link 的前后值；只影响本视图，其他历史 link 不改。ignore 不创建 link。已有相同 role link 不重复插入。

每个候选任务必须恰好出现一次映射；每个 update 候选步骤同样必须 new/update/ignore 恰好一次。一个旧任务不能在同次映射被多个候选写入/替代；一个旧步骤不能映射给多个候选步骤。未在映射中提到的旧任务、旧步骤保持原样，不自动标替代。

update 的 take_fields 是**替换白名单**，包含人工编辑字段时 UI 必须明确高亮“将覆盖你的修改”；用户确认本地预览即允许该次列明覆盖。未列字段绝不覆盖。evidence_ids、assumptions、needs_review 亦须显式选取，且引用均保留原 generation 和 analysis revision 的来源；不伪称新旧证据同一版本。update 不允许通过候选改 status/progress/completed_at/lifecycle。target_period 仅在 take_fields 明列时从候选更新；不因生成目标期不同而暗改原任务的整体目标范围。

步骤删除只能出现在 remove_existing_step_ids 的显式选择，服务显示完成/未完成及原记录；已完成步骤也可以经用户明确选择删除，但保留 tombstone 和事件。最终至少有一个未删除步骤。final_step_order 必须无重无漏列出最终所有存活的旧步骤以及被选 new 的候选步骤，不能用缺失顺序表达删除。新步骤自动赋永久 UUID；旧步骤状态保留。每次 add/delete/status change 会触发第 5 节进度规则。

### 3.3 预览、原子性与幂等

- CLI：dsheng task reconcile --run <run-id> --input-json <path|-> --dry-run --json；dsheng task reconcile --run <run-id> --input-json <path|-> --apply --json。相同输入 Schema 见 task-reconcile.schema.json。只调用本地服务，无 --confirm-send、无 Provider。
- --dry-run 校验并返回 diff、所有受影响 task revision 和 apply_token；不修改业务记录。UI 使用同一 prepare 服务。apply_token 为本地短期确认凭据，只绑定 workspace、candidate revision、分析 revision、规范化 reconcile 内容及当前任务 revisions；有效 10 分钟、不外发、不可记录全文，超时需重预览。
- --apply 必须同时传 --apply-token <token>；token 属于本地操作凭据，不写入源文件或业务日志。apply 前再次校验所有 revision 与输入 digest；同一计划变化、并发编辑、candidate 来源改变都使 token 失效。无需模型。
- mutation_id 由客户端生成。服务存 request_digest 和结果摘要：同 mutation_id+同 digest 重复提交返回原结果，不重复添加任务/步骤/事件；同 ID 不同 digest 返回 IDEMPOTENCY_CONFLICT。即使重复 apply token 已消费，同 mutation_id 也先命中已完成结果；若其结果因显式历史删除已被清除，返回 MUTATION_RESULT_DELETED，不重建已删除业务对象、不返回旧成功正文；未成功事务不得留下部分消费记录。
- expected_tasks 必须精确覆盖全部 update/replace 旧任务；同 workspace且revision相等。一个事务完成任务/步骤/links/events/候选 applied标记/幂等结果，任一字段失败全部回滚。相同 candidate 只能应用一次；用户不满意应重新生成新的 candidate，不能再次套用已消费内容。
- applied candidate 不允许再次变更映射；保留原 candidate JSON、model局部ID到永久ID映射、操作者来源 ui/cli、applied_at及应用结果。不把候选快照当当前任务字段读取来源。

## 4. 字段持久化、编辑和导出唯一来源

### 4.1 字段映射

| 模型输出/产品字段 | 当前态及永久历史 | UI/CLI 可编辑规则 |
| --- | --- | --- |
| task_key / step_id / order | candidate JSON原样保留；永久task/step UUID及step_order另外分配 | 模型局部key不可编辑/不可当主键；顺序使用step move或reconcile。 |
| title/summary/category/metric_id | tasks当前字段、task_revision完整快照 | 内容patch可改；category存规范码revenue/cost/efficiency/other，显示标签分别为销售最大化/费用最小化/时间最短化/其他事项；metric_id为本workspace有效metric_definitions UUID或null，不能填写metric_key/标签；指标version与已确认scope在analysis revision固定，null仅category=other。合并patch后的最终状态不满足此关系返回TASK_CATEGORY_METRIC_MISMATCH，不凭类别猜指标。 |
| source_period/target_period/cycle_role | task_period_links；target_period亦存当前task目标范围；旧link快照不跟随修改 | 通过period_link操作显式attach/update/detach；不能把同一task复制成两个角色实体。 |
| baseline/value/unit、target_value、planned_value、actual_value、current_value | baseline_json及各数值当前字段，计量/来源与revision同存 | baseline为分析来源只读；目标/预定/实绩/当前人工值修改需source_note；不得覆盖分析metrics，人工值单独标user_input。数值规则沿用指标合同。 |
| unit/mp_label/mp_unit | tasks当前显示单位及指标定义引用 | 可改label；更改unit必须与metric定义兼容，不能只改文字产生换算；不兼容返回UNIT_MISMATCH。 |
| owner/planned_start/due_date | tasks当前字段 | nullable；日期顺序校验，不推断缺日期。 |
| resource_need/success_criteria | tasks当前字段 | 内容patch可改，记录before/after。 |
| evidence_ids | task_evidence及task_revision里证据快照 | 仅来源选择/候选reconcile更新，普通自由文本patch不可伪造ID。保留source_id+analysis_revision+坐标，不仅文件路径。 |
| assumptions/needs_review | tasks当前JSON与生成快照 | 候选reconcile可替换；人工备注review_note追加，不通过删除提示掩盖原生成不确定性。 |
| smart_check | generated SMART JSON与smart_status、assessed_task_revision | 只读；新生成任务为generated，首次人工变动或混合reconcile后标stale。展示“生成时评价，内容已改待复核”；不静默再调模型。 |
| step.action/owner/start_date/due_date/done_criteria | task_steps当前字段与task revision | patch/add允许编辑；删除为tombstone；至少一条存活步骤。 |
| step.evidence_ids | task_step_evidence或steps evidence_json快照 | 只由合法源/候选映射更新，保留来源版本。 |
| step.status/completed_at/blocked_reason | status为唯一状态源；checked=(status=completed)派生 | 人工步骤状态patch；completed_at由服务时间写，重开置null；blocked必须非空理由。 |
| task.status/progress/progress_mode/reason/completed_at | tasks当前字段及events | 仅progress操作或确定性步骤规则更新；模型不修改执行状态。 |
| note/review_note | 当前说明与事件 | 可编辑，原事件不可改；不能把用户note升级为source evidence。 |

当前任务详情/编辑/当前列表/当前导出都读规范化当前态；生成原文面板只读candidate/generated snapshot。历史导出按第 6 节snapshot读取，绝不把旧模型输出覆盖用户已保存的内容。

### 4.2 编辑输入与校验

本地任务写入/SQLite/领域与导出JSON的 target_value、planned_value、actual_value、current_value、baseline.value 及 progress 使用规范十进制字符串或null。金额/数量/工时输入最多6位小数、28位有效数字，progress为0–100且最多2位小数；JSON number、指数、分组逗号、多余前导/尾零与负零均不能作为本地写入合同值。UI把明确输入经Decimal解析并规范化后调用核心；CLI JSON须直接遵守Schema。超精度/范围遵守数据合同，不通过f64转换或静默舍入。revision、order、计数等离散整数仍使用JSON integer。Provider输入/输出Schema仍采用number：传输时Decimal-aware序列化数字字面量，接收时保留数值词法直接解析Decimal，再生成本地规范字符串，原始候选快照保持原输出而非重新序列化浮点数。

分类边界采用固定exact映射，数据导入与Provider候选均先经相同适配器进入本地规范码：

| 本地规范码 | 接受的完整标签（逐项exact匹配） | 展示名 |
| --- | --- | --- |
| revenue | revenue、收入最大、收入改善、销售最大化 | 销售最大化 |
| cost | cost、费用最小、费用控制、费用最小化 | 费用最小化 |
| efficiency | efficiency、时间最短、时间/效率改善、时间最短化 | 时间最短化 |
| other | other、其他事项 | 其他事项 |

仅对首尾空白清理，不做模型/模糊分类。task-edit只接收规范码。导入未知类别停在人工字段映射，用户确认规范码后再提交；Provider未知类别按TASK_CATEGORY_UNRECOGNIZED拒绝整份候选并保留上一有效结果，不保存为可应用候选、不默认other、不静默修正模型正文。category与metric_id跨字段关系在合并patch后验证。生成请求category_options固定使用四个规范码，展示中文不影响身份。

所有 task update/step add/update/delete/move 都由 CLI/Tauri 适配为 task-edit.schema.json；content 与 progress 操作分开提交，符合“保存任务/保存进度”两个动作。一个 content 可同时含task patch和多个step operations，整体原子。客户端需读取当前revision再写；服务不自动填expected revision。

日期统一ISO YYYY-MM-DD；start<=due；task period 的from<=to；已给task整体边界时，步骤日期须在该边界内，否则提示先修改整体计划日期。null为未定，不以目标期或单日自动补齐。reconcile期间字段同样检查。UUID格式、ISO日期和非空理由校验使用format checker及trim后非空规则。

数值patch涉及target/planned/actual/current时必须在operation同时提供非空 source_note（注明人工调整依据）；这不是模型事实。修改metric_id/unit/目标、期次链接、标题/说明、责任/日期、资源/验收、步骤文字/结构/证据均标SMART stale。仅执行状态、进度、完成时间、普通note/review_note和排序不改变目标语义，不因此改SMART状态。

task evidence、assumptions、needs_review不允许用普通patch任意删除；候选合并的原始版本仍可查看。target_period作为任务内容patch显式修改整体目标期间，不自动改其他run的period link；已有步骤日期超出新范围则整个patch拒绝，用户需在同一content事务一起调整相关步骤。内容patch不允许status/progress/completed_at（这些用progress操作）；身份/创建时间/源revision/SMART JSON禁止客户端写。来源或指标已失效时可读历史，修改需重新选择有效指标但不能改旧证据历史。

## 5. 进度与状态转换表

### 5.1 自动模式

只计算未删除步骤，至少1条。progress=completed_count/total_count×100，使用十进制四舍五入到2位；完成判断用计数全等，不用展示舍入。真实未全完成但四舍五入恰到100时显示上限99.99并保留计数（例如99999/100000）。

自动状态优先顺序：

1. 有任务显式block reason或任何步骤blocked → task.status=blocked（可0–100%），不会因勾选其他步骤解除阻塞。
2. 无阻塞且全部步骤completed → completed、progress100。
3. 无阻塞，至少一步completed或in_progress → in_progress（不足100%）。
4. 无阻塞，全部not_started → not_started、progress0。

任务“已延期”是derived overdue=(workspace当地today>due_date && status!=completed && lifecycle=active)，不是独占状态；可同时“阻塞且延期”。due_date=null则overdue=false且显示未设截止；日期到期当日未结束不算延期。timeline存事件UTC时间并按workspace时区显示，不以展示时区更改历史event时间。

### 5.2 人工模式与阻塞

manual进度必须提供规范十进制字符串0–100（最多2小数）及非空adjustment_reason。status约束：not_started仅0；completed仅100；in_progress为0<=p<100；blocked任意p但blocked_reason非空。manual允许用户说明为何步骤完成比率与总体评估不同，UI显示两者，不把人工总体100%反向勾全步骤。

下一次步骤status变化、添加或删除步骤时，自动切回auto并按上述计数重算，写progress_override_reset事件；仅改步骤文字、责任、日期、排序不切换manual。任务显式blocked原因持续保留，直到用户通过progress操作明确选择非blocked且无blocked步骤；未解除的步骤阻塞不能被总体状态覆盖。

auto progress请求中的progress与adjustment_reason必须null；服务计算结果，用户提交的status必须与自动结果一致，或显式请求blocked并提供原因。要解除显式block，用户提交非blocked、blocked_reason=null；如果仍有blocked步骤返回STEP_STILL_BLOCKED。UI先展示计算值再提交，不能自动帮用户解除。

从非completed变completed：completed_at写本地服务当前UTC时间；重复相同completed保存不重写时间。从completed到其他状态：completed_at=null，追加reopened事件并保留旧completed事件。步骤同理；task因新增步骤/取消勾选变未完成，也写reopened。lifecycle=superseded不可再推进，须新建独立后续任务，不复活旧任务隐去替代记录。

每次更改包括field before/after、revision、actor_source(ui/cli/import/generation)、occurred_at、mutation_id、reason；事务失败不得留下半个事件或部分步骤。

## 6. 跨期关联与稳定历史导出

- task_period_links绑定task_id、run_id、analysis_revision_no、cycle_role、source_period、target_period。允许同一task在不同run的plan/review中出现；相同task/run/revision/role唯一，不复制tasks与events。
- 同一分析本期是review窗口、下期是plan窗口；用户明确attach关联，自动预选仅根据已确认目标窗口精确重叠，不能靠标题/模型猜身份。attach不重置task目标/进度。更新link只影响此视图，改task整体目标须显式内容/目标期操作并标SMART stale。
- 新的analyze revision复制旧revision已确认的task links作为新版本视图基线，引用同一永久task，写继承来源；旧links不更新。重生成candidate引用当前revision；跨期既有任务存在即进入review_required。
- analysis revision输入与指标永不改；任务在该revision仍当前时可继续推进。每次影响该run当前链接任务的本地事务，追加run_task_snapshots：task_snapshot_no、captured_at、event_cutoff_seq、完整task/steps/evidence/link的必要快照及每项task_revision。只含规范化输出，非整表源数据。
- 新analysis revision事务同时创建task_snapshot_no=0，包含继承后完整links及当时任务当前态；无任务时是显式空snapshot。之后每次相关事务递增snapshot_no。旧revision退出当前时，封存它最后一个task_snapshot；以后任务继续推进只更新当前关联run的快照，不追加到已封存旧revision。
- 历史查看/导出标识为run_id+analysis_revision_no+task_snapshot_no。首次选择revision未给snapshot时，只解析一次到该revision最高已保存snapshot并返回其ID和时间；后续图表/导出按钮绑定这个ID，不能每次刷新偷换。用户可明确选择更晚snapshot或当前任务视图。任务后续进度不会改变已选snapshot或已生成导出。
- CLI：history show --run <id> --revision <n> [--task-snapshot <n>] --json；export同样接受--revision/--task-snapshot并在manifest输出解析到的快照。task show/timeline默认展示永久任务当前态；其历史不是“实时任务数据”。
- 导出时不存在可读源文件也能读取规范化分析和任务快照；续问/重新生成仍需原来源校验，不从历史快照伪造已选整表输入。

## 7. SQLite 增量字段/表合同

此为逻辑表规范，不是此轮执行migration。所有跨workspace外键在服务和数据库约束中一起检查。整数revision正值；原immutable记录不可更新，删除流程除外。

| 表 | 必须增补/改定的字段与索引 |
| --- | --- |
| analysis_runs | current_revision_no、analysis_input_fingerprint；workspace+period_key唯一。 |
| run_revisions | run_id+revision_no唯一且含每版包括当前；analysis_input_fingerprint、canonicalization_version、comparison_base_refs_json、source_selection_snapshot_json、action_period_snapshot。 |
| generation_sessions/attempts | analysis_revision_no、analysis_input_fingerprint、generation_input_fingerprint；session固定，attempt确认快照固定。 |
| diagnosis_reports | analysis_revision_no、analysis_input_fingerprint、report_revision_no；每个报告revision有不可变report_revision snapshot；引用按两种revision验证。 |
| generated_outputs / task_candidates | candidate_generation_id、candidate_revision_no(首版固定1)、run/revision/input fingerprint、output_json、state(candidate/applied/discarded)、applied_at；候选正文不可改。 |
| tasks | workspace_id、origin_run_id/origin_revision、task_revision_no、lifecycle、superseded_by_task_id、target_period_json、baseline_json、unit、resource_need、success_criteria、assumptions_json、needs_review_json、smart_check_json、smart_status(generated/stale)、assessed_task_revision、progress_mode、adjustment_reason、blocked_reason、review_note。移除UNIQUE(run_id,model task_key)作为身份约束；model局部key留candidate。 |
| task_steps | UUID id、task_id、step_order、action、owner、start_date、due_date、done_criteria、status、blocked_reason、completed_at、deleted_at；存活step_order在task内唯一；checked派生，不保存第二真值。 |
| task_revisions | task_id+revision_no唯一；fields/steps/evidence完整规范化快照、actor_source、mutation_id、created_at。 |
| task_period_links | UUID id、workspace_id、task_id、run_id、analysis_revision_no、cycle_role、source_period、target_period、inherited_from_link_id；task/run/revision/role唯一。 |
| task_events | task_revision_no、actor_source、mutation_id、event_seq（全库单调）、before_json、after_json、reason，事件不可改。 |
| task_evidence / task_step_evidence | owner task/step、source_id、analysis_revision_no、evidence_id、规范化证据摘要及坐标；不能仅FK到会被重做替换的source_files当前行。 |
| run_task_snapshots | run_id+analysis_revision_no+task_snapshot_no唯一、task_snapshot_json、event_cutoff_seq、captured_at、sealed；immutable。 |
| task_candidate_applications | candidate_generation_id唯一、mutation_id、mappings_json、permanent_ids_json、applied_result_json。 |
| local_mutations | workspace_id+mutation_id唯一、request_digest、result_json、affected_run_ids/affected_task_ids、result_deleted、committed_at；不保存apply token全文。 |
| app_schema_metadata | schema_version、migration_state、last_migrated_by_app_version；应用binary定义supported read/write schema范围。 |

任务内容和步骤不能只保存在generated_outputs，否则人工编辑无法成为当前态。新表可用JSON列承载复杂规范化对象，但字段合同、版本和唯一读取来源不得随实现者自由变化。

## 8. 历史删除边界

删除指物理删除数据库中目标run及其run-local数据，不伪装为归档。CLI先history delete --run <id> --dry-run --json返回delete_preview_id/范围/阻断依赖；真正删除须--confirm-delete <delete_preview_id>。UI同样二次确认。预览10分钟有效，绑定当前run revision及依赖集合hash，提交前重新核对；变化则重新预览。

以下任一存在则返回DELETE_DEPENDENCY_BLOCKED：

- 目标run/session有活动generation attempt。
- 任何仍可推进的任务（lifecycle=active且status!=completed）的origin_run或period link引用该run。不会为方便删除自动完成/替代任务。已完成且仅属目标run的任务可进入下述明确删除清单，不强迫先制造替代任务。
- 其他run的任何revision/比较、task evidence、报告或生成结果以结构化引用依赖目标run任何revision；跨期已替代任务的存活引用也不级联删除。
- 目标run中origin的superseded task仍被另一任务superseded_by关系、其他run link或外部证据引用。

无阻断时预览列出并事务删除：run/current metrics/mappings/source metadata、所有run revisions/comparisons、该run独立reports/sessions/turns/attempts、candidate/generated outputs、task_candidate_applications、run_task_snapshots及period links。只属于目标run、已completed或superseded且无存活外部引用的task可连同其steps/revisions/events/evidence删除；在预览逐项计数，用户看到不会误以为保留这些历史。同事务清除引用这些已删业务对象的 local_mutations.result_json，标 result_deleted=true，只保留 mutation_id/request_digest/时间等最小防重放元数据；重试返回 MUTATION_RESULT_DELETED，不复活已删对象。删除前一次最终依赖检查和删除同一事务，不能先检查后另事务删。

不删除磁盘原始XLSX/CSV/PPT、Provider配置、workspace、其他run/任务/事件或已导出的文件。数据备份、外部导出可能仍含已删除记录，UI/CLI明确说明“从当前数据库删除，不清除既有备份/导出”。首版无静默全库清空和绕过依赖的force参数；用户若要清理整企业须另行规划，不把history delete拓展成此功能。

## 9. 共用数据库、升级与恢复

- 桌面与独立CLI使用相同用户目录数据库；产品元数据中记录schema_version。每个binary编译时声明supported_read_schema_min/max与supported_write_schema；首次开库先读版本，不支持的新schema立即返回DB_SCHEMA_UNSUPPORTED并提示升级对应客户端，不尝试写入或自动降级。不兼容旧库只有最新版迁移入口可升级。
- 升级前要求所有已登记GUI/CLI实例退出并确认没有active generation；PID须核对启动时间/实例nonce，不能只凭可复用PID误判。持有跨进程migration lock，再以SQLite独占事务检查版本，锁忙即返回DB_MIGRATION_BUSY，不无限等待、不杀其他进程。
- migration前使用SQLite一致性backup API创建同用户数据目录 backups/pre-migration-<schema>-<timestamp>.sqlite；不可只拷贝WAL模式主数据库文件。检查backup integrity、权限与文件大小，失败则不执行migration。备份包含明文Key及本地业务历史，UI/CLI明确路径和保密属性；不自动外传、不进入软件发布包或业务导出。备份失败返回 DB_BACKUP_FAILED。
- 单一迁移事务应用所有升级步骤、验证关键约束，最后更新schema_version并commit；失败rollback，保持原库可由旧版本读取，保留错误元数据与备份。失败返回 DB_MIGRATION_FAILED；禁止迁移时联网/调用Provider。迁移SQL不能直接在工作文件上进行不可回滚的外部数据删除。
- 断电/崩溃后首次启动检查SQLite完整性和migration marker：事务未提交由SQLite恢复原版本；已提交则新版本；不能猜半迁移。若integrity失败，返回 DB_INTEGRITY_FAILED，停止普通写入并提示从已验证本地备份手动恢复，不自动覆盖现库。
- 恢复是显式本地操作：所有实例退出、验证backup版本/integrity、再次确认覆盖目标；先对现库做一致性保留备份，再原子替换且校验，不删除唯一可用副本。恢复失败返回 DB_RESTORE_FAILED 并保留现库与备份；restore不改变源业务文件，也不自动发请求。旧schema备份由兼容binary打开或按相同迁移流程升级，禁止自动降级现库。
- 卸载、覆盖安装、更新默认保留用户数据和backups；便携包使用同一用户数据目录，不把“便携”解释为数据库放安装目录。首版不自动清理backups，手册给用户显式本地删除说明；history delete不清理备份。

## 10. 必须新增的验收与主 Agent 交接

本合同的合成领域用例在 task-state-cases.json；它们不是伪Provider响应，不调用网络。需要工程实现后按相同输入/预期自动化：

- 身份：模型局部key变化、同指标同标题但不同动作、重复candidate apply、同mutation不同payload、跨workspace ID全部覆盖。
- 合并：保留未选人工字段/完成步骤、显式覆盖高亮、未映射旧任务不变、显式删除至少一条、候选空数组、全事务失败回滚。
- 版本：只改手工输入仍使指纹变；请求途中重做、旧诊断引用、基期重做不改旧同比；Provider revision变化按配置合同联测。
- 状态：1/3完成33.33、极小未完成不误100、manual被步骤变化重置、block优先、解除step block、重开时间与事件、延期派生及缺日期。
- 历史：同task跨plan/review仍同ID；旧revision和已选task_snapshot不随live推进漂移；源文件离线仍可查快照。
- 删除：活动请求、未完成active task、跨期基期引用均拒绝；无依赖仅删预览范围；原文件/Provider/备份不动；并发依赖变化使预览失效。
- 升级：旧CLI拒绝新版库；migration忙不强杀；备份失败不迁；事务失败rollback；重装保留数据；恢复前保留现库并检测版本/完整性。

规则已在本合同选择唯一行为，工程线程只实现及验证。动态业务内容仍由独立提示词内容门禁决定，本文不能替代真实模型验收。主 Agent 集成时将本合同对应验收追加在现有1–64之后，并修正主计划旧概述，不要求实现线程自行协调冲突。
