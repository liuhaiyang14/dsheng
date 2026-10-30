# 数据、周期、模板与输出合同 v1

状态：**规则定稿，工程未实现**。日期：2026-10-30。关闭 PR-01、PR-02，并规定 PR-11 的导入/导出布局。本文件不包含真实企业数据；机器可读指标与合成算例分别见 [metric-catalog.json](../../contracts/v1/metric-catalog.json)、[data-period-cases.json](../../contracts/v1/data-period-cases.json)。后两者是纯领域合同，不是 Provider 响应。

## 1. 数值、指标与计算

### 1.1 数值合同

- 运算采用 Decimal，至少28位有效数字；输入金额、数量、工时、单价最多6位小数。超过输入精度报 `decimal_precision_exceeded`，超范围报 `decimal_overflow`，不静默截断。XLSX数字优先读取XML原始字面量；读取库只给float且不能恢复精度时标 `precision_unverified`。禁止以f64中转精确金额。
- 加减乘保持上述精度；除法保留12位小数、round-half-even。中间结果不按展示位数先舍入。金额显示2位、数量/工时最多6位、百分比1位，展示也用half-even。SQLite及业务JSON采用规范十进制字符串（无千分位/指数/多余尾零，负零为0）；现有模型schema的number字段由decimal-aware序列化器输出数字字面量，解析同理，不将字符串直接塞进模型number字段。
- 值状态为 `valid|missing|pending_input|invalid|conflict|configuration_required|unsupported_formula|precision_unverified`。只有valid进入经营计算。空单元格为missing，`待补/待确认`为pending_input；`NA/N/A/-`保留原文本并记missing_token，不自动改为零。0及负数原样保留。
- 千分位仅接受三位分组（1,234.56），不猜逗号小数点。表头/单元格明确`%`才能除100。金额单位元/千元/万元按1/1000/10000归一到元；币种和含税口径必须来自明确来源或工作区确认。不同币种不自动换汇，不同税口径不自动折税。
- `direction=up|down|neutral`表达改善方向；差异统一 `actual-planned`，另给favorable（up正、down负、neutral不判）。不能把down指标的达成率翻转成计划÷实际。

### 1.2 指标注册与有限公式

每个指标必须固定 `metric_key/label/semantic_type/unit/direction/target_constraint_scope/aggregation/formula/required_inputs/denominator_policy/comparison_policy/version/effective_from`。口径、公式、单位、方向或比较政策变化创建新版本；显示名不改变semantic hash。

| 类型 | 唯一聚合方式 | 禁止行为 |
| --- | --- | --- |
| flow | 同口径、已证明不重叠的完整明细或期间值求和 | 明细与汇总同时加，累计列当新流量 |
| stock | 选定期末已证实余额，或期初+完整净变动 | 月末余额求和/平均，缺期初当零 |
| ratio | 汇总原分子/分母后重算 | 百分比求和/简单平均、缺底数反推金额 |
| configured_derived | 按确认的AST对已聚合输入重算 | 不认识的公式直接相信缓存 |

收入`monthly_revenue_floor`仅适用于明确收入语义、direction=up、月度目标且可比的指标。catalog仅sales_net默认采用；企业新增收入指标须显式声明。无冲突或已解决且明确选定的年度当月分解/去年同月实绩构成可比下限；候选目标取max（合法显式目标、有效年度当月分解、去年同月实绩）。若显式目标低于下限，保留原目标输入不变，模型可建议上调候选至下限，但必须needs_review注明“候选上调待采纳”；它不是已批准目标。未解决来源冲突的候选值完全不得参与max。费用、时间、成本及中性指标为explicit_target_only：仅取明确确认的计划/改善目标，不用收入下限，不自行改成min，不发明降幅。历史实绩不等于目标。非月度/无可比下限只用显式目标，否则null；年度总计划不自动除12。

结算收益、结算收益率、单位时间附加值无通用默认公式；企业必须确认 `labor_treatment=included_in_expense|separate_from_expense`、科目输入、单位及AST。缺配置时configuration_required；导入公式说明只作候选说明，不自动执行。这是运行时口径配置，不要求工程师选择会计定义。

AST仅允许ref、const、add、sub、mul、div；每节点字段精确匹配catalog，二元运算，最多64节点、深度16，无环，仅引用工作区注册指标。add/sub单位相同；mul/div推导单位，最终须等于声明；const为无单位decimal。禁止代码、任意Excel公式、动态文件/网络、宏或外链。所有div分母须>0，0返回zero_denominator，负值negative_denominator；分子可负，比例可大于100%。退货/冲销保留负流量，但方向/原交易引用须映射确认，不凭金额正负猜借贷。

### 1.3 库存与资金差额

- 库存量=期初+入库−出库，要求同一货品/仓库/单位，明确期初截至时间以及之后完整流量。仅明确证明从建立起完整累计且期初0，才可输入有依据的0。无期初及已证实期末则不能计算。
- 已证实期末数为closing_balance观察，与rollforward冲突时人工选源；负库存允许保存并警告，不改0。
- 参考结存金额逐货品/仓库用库存量×参考采购单价，再加总；禁止总量乘未经确认均价，不宣称会计存货成本。组成缺失时仅显示已知部分及缺项。
- 开票−收款叫“开票收款差额（期间）”；入库−付款叫“入库付款差额（期间）”。它们可能跨期，不能自动叫应收/应付余额或逾期欠款。余额须另配置期初+完整变动的stock指标；一期不推断账龄。旧“未回款/未付款”标签须人工确认定义，不自动等同余额。销售净额及订单成本的毛利公式只接受相同已确认不含税口径；其他税口径不得自动折算后代入。

### 1.4 指标定义到模型输入的投影

模型输入不是第二套计算口径。task-input 与 diagnosis-input 共用以下确定投影；应用先完成计算/来源选择/可比性检查，再组装请求，模型不能自行补齐这些决定。

- `metric_id`引用工作区内稳定的`metric_definitions.id`（UUID），`metric_key`是catalog业务键，不能拿显示名称或分析结果行号替代ID；版本变化保留身份并在analysis revision冻结具体version与semantic hash。`metrics`按metric_id排序，同一请求每定义只有一条观察。多业务scope同时存在时须先选择一个scope，或显式确认同量纲、无重叠的聚合scope；未定返回`SCOPE_SELECTION_REQUIRED`，不把两个货品/仓库的同名指标当同一值。定义投影保存所选scope_key。
- `workspace.metric_definitions`只发送本次使用指标及其公式依赖的冻结定义。每项字段固定为`metric_id/metric_key/version/semantic_hash/label/semantic_type/unit_signature/direction/target_constraint_scope/revenue_semantics/aggregation/formula/required_inputs/denominator_policy/comparison_policy/effective_from/scope_key`；无公式为null，未配置项保留configuration_required于输入异常，不发送虚构公式。metric_key仍用于AST引用，required_inputs依catalog原键；不能把显示中文改成AST键。
- `metrics.label/direction/target_constraint_scope`来自冻结定义；后端只有在明确`revenue_semantics=true`且direction=up时允许monthly_revenue_floor。`unit_signature`保存解析后的dimension/base_unit/currency/tax_basis；其中workspace_confirmed/explicit_per_metric占位必须在发送前替换成实际确认值。模型的单个`unit`字符串固定为unit_signature的规范JSON字符串（键按字母序，null不省略），因此不同币种/税口径不会只以“元”混同；所有target候选和模型返回baseline/unit均使用相同字符串，UI另显示友好单位。ratio单位为dimension=ratio、base_unit=1，值0.15即15%，不把0.15改为15传输。
- `metrics.target`仅为当前来源期间已确认计划值，`actual`为该期间实绩，`delta=actual-target`；缺任一操作数则delta=null，不把目标窗口的候选目标混进当前预实差。`explicit_target`是目标窗口显式确认计划；`annual_allocated_target`是目标窗口对应月份的annual_month_plan；`prior_year_actual`是目标窗口去年同月可比实绩。三者均为候选对象或null，不以本期实绩代替去年同月、不以年度总计划除12。
- 候选对象的value来自单一已确认观察，unit必须与metrics.unit完全一致，evidence_ids非空；comparable根据§3核定。未解决冲突不能制造一个候选value，候选置null并将双方保留于source_conflicts；已解决冲突仅投影获选value、conflict_status=resolved及真实resolution_id，未冲突为none且resolution_id=null。无效候选保持缺项原因，不凭历史缓存回填。
- `mom/yoy`不存在可请求的基期时为null，原因进入missing_or_anomalous_inputs。已确定基期时固定对象字段`comparison_type/label/base_from/base_to/base_value/absolute_change/change_rate/availability_status/reason/base_refs/evidence_ids`；类型为mom或yoy，数值按§1.1模型边界序列化，缺值为null；availability_status为available/partial/unavailable，reason在available时null，部分覆盖或不可比时给出§3的确定原因；base_refs为run_id+revision_no数组，来源表内比较无run则为空但保留evidence_ids，不伪造历史记录。百分比不可算但绝对差可算的zero_base/negative_base为partial。
- 本地SQLite、CLI及编辑合同中的十进制字符串，只在模型序列化边界转成number字面量；输出number由十进制解析器还原字符串再参与本地计算，不经JavaScript Number/f64中转。任务进度的本地序列化与0–100/两位小数边界按task-edit合同；revision/order等计数仍为integer。

#### 投影类型与完整性

以下对象所有列出的字段均required、additionalProperties=false；nullable字段仍必须显式传null。ID集合唯一；所有日期启用format=date并校验真实公历日。模型边界number须有限十进制字面量，不接受NaN/Infinity。

| metric definition字段 | 类型/约束 |
| --- | --- |
| metric_id | string，format=uuid；工作区稳定定义身份 |
| metric_key | string，pattern=`^[a-z][a-z0-9_]{0,63}$` |
| version | integer，minimum=1 |
| semantic_hash | string，pattern=`^[a-f0-9]{64}$` |
| label、scope_key | string，minLength=1 |
| semantic_type | enum flow/stock/ratio/configured_derived |
| unit_signature | 下述unitSignature对象 |
| direction | enum up/down/neutral |
| target_constraint_scope | enum monthly_revenue_floor/explicit_target_only/none |
| revenue_semantics | boolean；true才有资格配置monthly_revenue_floor |
| aggregation | enum sum_disjoint/configured_recompute/recompute_after_aggregation/ratio_of_aggregated_inputs/opening_boundary/closing_balance_or_rollforward/closing_boundary/sum_item_closing_values |
| formula | null或§1.2递归AST；ref.metric_key同上pattern，const.value为规范decimal字符串，运算args恰2项；节点/深度限制另做领域校验 |
| required_inputs | string[]，各项metric_key pattern，uniqueItems=true；可为空 |
| denominator_policy | const positive_only |
| comparison_policy | object，仅positive_base=relative_change、zero_or_negative_base=absolute_only，两字段required |
| effective_from | string，format=date；catalog的初始确认日期占位在本地注册时替换 |

unitSignature四字段均required：dimension为money/time/quantity/ratio/money_per_quantity/money_per_time；base_unit为非空string；currency为ISO大写三字母string或null；tax_basis为inclusive/exclusive/not_applicable或null。money及其派生维度currency须非null且tax_basis须为已确认枚举，其他维度二者为null。ratio的base_unit固定1、time固定hour，其余使用已确认规范单位。任何placeholder/未知币种税口径不得进入有效模型metric；相关metric排除并在异常中标configuration_required，原始来源仍按已选范围保留。模型unit字符串必须等于上述对象的规范JSON序列化值，不允许同义缩写。

semantic_hash按冻结定义的metric_key、semantic_type、unit_signature、direction、target_constraint_scope、revenue_semantics、aggregation、formula、required_inputs、denominator_policy、comparison_policy，以及本地确认的labor_treatment（无此配置时null）和dependency_semantic_hashes计算canonical JSON SHA-256。dependency_semantic_hashes是直接AST引用与required_inputs并集的metric_key→semantic_hash映射，按键排序、自叶向根计算；无依赖为空对象，引用环/缺依赖拒绝。这样底层科目口径改变时，使用相同公式文本的派生指标hash也会改变，不误判跨期可比。labor_treatment与dependency_semantic_hashes是本地冻结定义的哈希材料，无需扩大模型definition投影字段；依赖完整定义已经随workspace.metric_definitions发送。不包含身份ID、version、label、effective_from、scope_key，scope由比较签名单独绑定。劳动费用处理必须与确认AST的费用包含关系一致，变化必重建定义版本；不要求模型猜劳动费用处理。

| comparison字段 | 类型/约束 |
| --- | --- |
| comparison_type | enum mom/yoy，必须与外层字段一致 |
| label | string，minLength=1；week的yoy固定52周同期 |
| base_from、base_to | string，format=date，from<=to；不能形成基期时整个mom/yoy为null |
| base_value、absolute_change、change_rate | number或null，保留上述空值/正基数规则 |
| availability_status | enum available/partial/unavailable |
| reason | null或枚举zero_base/negative_base/missing_current/missing_base/coverage_incomplete/definition_mismatch/unit_mismatch/scope_mismatch/calendar_mismatch/source_conflict/configuration_required/unsupported_formula/precision_unverified/invalid_input；多异常按下述优先级选一个，完整列表仍存missing_or_anomalous_inputs |
| base_refs | array，uniqueItems=true；每项仅run_id(UUID string)、revision_no(integer>=1)，均required；按run_id/revision_no排序，可空 |
| evidence_ids | string[]，minLength=1、uniqueItems=true，可空；只有当前获准输入已有的证据ID |

available表示比较两种结果均有效，reason=null且三个数值非null；partial仅用于zero_base/negative_base，base_value和absolute_change非null而change_rate=null；其余为unavailable且两种变化均null，base_value可保留已确认但不可比的已知值。异常优先级固定为source_conflict→configuration_required→unsupported_formula→precision_unverified→invalid_input→definition_mismatch→unit_mismatch→scope_mismatch→calendar_mismatch→coverage_incomplete→missing_current→missing_base。无合法日历基期（date_not_in_prior_year/prior_year_range_not_comparable/partial_period）时比较对象为null，原因记录于异常列表。不存在历史run仍可用有证据的来源比较列，base_refs=[]；禁止据此假造run ID。

#### 来源单元格与人工输入证据投影

`sourcerecord`与手工输入是两种不同证据域：前者可定位到用户确认的本机文件单元格，后者是当前用户的声明/数值录入。二者可以用于已确认公式和目标计算，但来源等级不可互换。

**source_records最小结构：**一个记录=用户最终选择范围内一个非空物理单元格；不发整行、整张sheet或原文件二进制。每对象固定字段`evidence_id/source_id/file_label/sheet/cell_or_row/values`，全部required、additionalProperties=false；sheet可null（CSV）；`cell_or_row`为单一A1坐标，如`C7`（CSV按RFC4180逻辑记录定位，列仍按A1字母，如`C7`）；不展开多个坐标或范围。即使一行有多个值，每个格独立记录。标题/注释/公式格亦一格一记录。合并非空单元格只发左上锚点一次，保留merge范围；不可把锚点复制到其余物理格。values不包含绝对路径、公式外链目标或文件属性隐私。

`values`是封闭对象，全部字段required且typed_value/formula可显式null：

| 字段 | 类型/约束 |
| --- | --- |
| cell_type | enum text/number/formula/boolean/error/blank |
| raw_text | string；从单元格原文、数值XML字面量或公式源保留，不裁剪 |
| typed_value | string/boolean/null；数值为规范十进制字符串、日期先按§3/工作簿日期系统解析成ISO日期字符串；text原样、boolean布尔；formula/error/空格null |
| formula | string或null；只保留该格自身公式原文，不执行、不展开外链；普通格null |
| value_state | enum valid/missing/pending_input/invalid/conflict/configuration_required/unsupported_formula/precision_unverified |
| number_format | string或null；Excel原始显示格式，仅用于辨认percent/日期及复核，不作为业务值 |
| merged_range | string或null；合并锚点的完整A1范围，否则null |
| mapping | null或封闭对象`metric_id/field_key/measure_role/period/scope_key/unit`；对象字段全required，metric_id为metric-definition UUID或null，field_key为§4.2列出的一个标准字段，measure_role为actual/plan/annual_month_plan/metadata，period为已解析闭区间或null，scope_key为非空string或null，unit为规范unitSignature JSON字符串或null。任一非空field_key只表示单格经用户确认/唯一适配器映射后的角色，不表示同行其他格也自动继承。actual/plan/annual_month_plan必须metric_id、period、scope_key、unit全非空且role分别映射actual/plan/annual_month_plan；metric_label可含metric_id而period/scope/unit为空、role=metadata；其他描述性字段role=metadata、metric_id可null。未分类/未确认的文本不猜mapping，mapping=null。 |

blank空格不造记录；数值0是非空且valid。`待补/待确认`、NA标记、公式缓存、错误、不可恢复精度严格应用§1.1状态；formula永不执行，unsupported_formula不把缓存结果写入typed_value。原表显示格式percent且本格底值0.5时，映射比例的业务值为0.5（百分数），显示为50%；原始raw_text/number_format仍可审计。普通number为0.5时不擅自乘100。

顶层类型固定为：evidence_id=`cell:<source UUID>:<sheet-or-csv>:<A1>` string；source_id为UUID string；file_label为用户确认的非空显示标签（服务校验不含绝对路径）；sheet为非空sheet name或null；cell_or_row为一个规范A1 cell（列A–XFD大写、行1–1048576无前导零）或CSV逻辑记录对应的A1坐标；values按上表封闭结构。sheet-or-csv取已规范sheet名或字面`csv`；Excel禁止sheet名冒号，同一source/sheet/格只有一个ID。按`source_id/sheet/cell_or_row/evidence_id`排序。file_label不得包含绝对路径。来源hash/选定范围仍由source_manifest绑定；来源值/地址改动即使evidence_id相同也会因manifest和analysis revision变化失效。

**范围与预算：**只发送用户预览确认的文件、sheet、矩形范围内非空格；依赖公式若需要引用范围外单元格，不自动扩范围，只标unsupported_formula或要求扩大范围再确认。系统生成来源记录后估算最终完整请求UTF-8字节及预算；超过§provider-config-delivery硬上限，必须停止并告知需要缩小所选范围再重新预览，不能抽样、按价值排序删行、只发汇总、分块多请求或丢失ID。

**manual_inputs最小结构：**每项固定`input_id/metric_id/value/unit/confirmed/source_note/evidence_ids/input_revision/measure_role/period/scope_key/raw_text`，全部required、additionalProperties=false；input_id/metric_id为UUID，value为规范十进制字符串或null，unit为规范unitSignature JSON字符串或null，confirmed为boolean，source_note首尾清理后非空，evidence_ids恰有一个虚拟ID，input_revision为正整数，measure_role为actual/plan/annual_month_plan，period为闭区间或null（null仅允许经确认的永久定义配置，不得映射经营观察），scope_key非空，raw_text为用户原始输入字符串。应用赋予evidence_id=`manual:<input_id>:r<input_revision>`，虚拟source_id=`manual:<input_id>`。这两个ID只属于人工证据命名空间，不能放入source_manifest或source_records；增加/改值/单位/期间/范围/来源说明递增input_revision，旧快照可追溯。

模型上下文中manual evidence投影为现有evidenceRef四字段：`evidence_id=manual:<input_id>:r<input_revision>`，`source_id=manual:<input_id>`，`source_label=用户手工输入（第<input_revision>版）`，`locator=用户录入·<metric_id>·<period或工作区口径配置>`。它支持标记`user_report`的精确用户录入内容和已显式确认目标/计算依据，但不能伪装为原表单元格source fact；其`evidence_id`禁止作为`warning.resolution_requirement=source_record`风险的独立书证解除依据。组合依据若同时含manual和文件证据，警告解除须仅以实际支持风险的文件source evidence落证，manual不替代缺少的文件事实。模型若仅凭人工值陈述，`facts_and_hypotheses.fact_status=user_report`、按既有提示词保存本轮原文，禁止写作已独立验证的文件事实。

#### source_conflicts最小投影

仅数值观察之间的来源矛盾进入source_conflicts；缺失、无效文本、单位未知或未选scope分别进输入异常/映射待确认，不能用value=null伪造数值候选。每对象仅含以下字段且全部required，additionalProperties=false：

| 字段 | 类型/约束 |
| --- | --- |
| conflict_id | UUID string；应用分配，分析revision内唯一，非模型ID |
| metric_id | UUID string；引用本次冻结定义 |
| measure_role | enum actual/plan/annual_month_plan；实际日期范围区分本期实绩、目标期计划、去年同期实绩 |
| period | string，固定YYYY-MM-DD/YYYY-MM-DD，pattern=`^[0-9]{4}-[0-9]{2}-[0-9]{2}/[0-9]{4}-[0-9]{2}-[0-9]{2}$`；两端必须是真实公历日期且from<=to。JSON Schema负责形状，离线合同校验器及运行时领域校验负责日期语义。 |
| scope_key | string，minLength=1；与该metric定义投影相同 |
| unit | string，minLength=1；与该metric规范unit字符串完全相等 |
| candidate_values | array，minItems=2；每项仅value(number)、evidence_id(非空string)、source_label(非空string)，全部required；evidence_id在组内唯一且在本次获准来源中 |
| resolution_status | enum unresolved/resolved |
| resolution_id | UUID string或null；resolved必须非null且指向本地已确认解决记录，unresolved必须null |
| selected_evidence_id | 非空string或null；resolved必须等于本组某一candidate.evidence_id，unresolved必须null |
| resolution_note | 非空string或null；resolved为用户选择理由（trim后非空），unresolved必须null |

相同metric_id/measure_role/period/scope_key/unit只投影一个冲突组；同值候选合并的全部证据由本地观察保存，候选evidence_id使用稳定代表ID，获选观察的全部证据进入目标候选evidence_ids。不允许从两个不同值造均值作为selected值。resolved只代表本地已确认选择，不表示所有来源一致；保留落选候选供模型理解来源差异。输入candidate.conflict_status=resolved时，其resolution_id须对应这里的resolved组，value等于获选候选value，且evidence_ids包含selected_evidence_id；measure_role/period/scope/unit须能精确映射到explicit_target/annual_allocated_target/prior_year_actual。该关系在发送前本地校验，任何不符返回INPUT_CONTRACT_INVALID，不交模型裁决。无冲突的目标候选resolution_id=null。

## 2. 来源去重、覆盖与冲突

标准观察记录包含source_id/hash/ref、metric_key/version、measure_role(actual/plan/annual_month_plan)、scope_key、period_from/to、as_of、record_kind、unit_signature、value/state、coverage。record_kind为detail/period_total/cumulative_total/closing_balance/opening_balance。

1. 同内容hash+同sheet/选定范围重复扫描只解析一次，保留路径别名；不同范围不直接去重。
2. 相同metric/role/scope/unit/tax/完整期间的多个汇总为候选同一观察，不能相加。值相同保留一个经营值及所有证据；不同则conflict、经营值null，保留双方，用户选主来源并填原因后才valid。
3. 明细与汇总共存，预览必须选择authoritative representation；只有一种才自动选择。明细不是默认优先。未选为source_selection_required。未选汇总仍可对账，默认容差0。
4. 明细去重须确认业务行键（如document_id+line_id），单据号不等于行键。同键同全部业务字段可去重；同键不同值冲突。无键同值不代表重复；重叠来源无可靠键为overlap_unresolved，须选源或明确不重叠范围。
5. 累计列须有累计起止，作为独立范围汇总；不能与其内部月份重复相加。首版不自动用累计差分制造期间数据。总计/累计不删除，可作合法范围观察或对账。父科目和叶子不可同时求和。
6. 手工补录与来源相同指标也遵守候选/冲突流程，不自动覆盖。解决记录保存观察IDs、原因、操作者、时间、run revision；改选择创建新revision。派生输入冲突仅阻断相应派生项，其他指标继续。

coverage包含from/to/granularity/status(complete/partial/unknown)/confirmed_by/empty_days_are_zero。文件名不证明完整性。完整交易流或用户显式确认完整范围后才能聚合为完整周/月/季；空日仅当complete且empty_days_are_zero=true可作0，默认false。月度完整三个月可聚财季；季度不能拆月、月度不能拆日/周、文件级汇总不能拆趋势。ratio需底数、stock需期末。日与月重叠仍选一种表示。趋势至少2个有效桶；缺桶null断线，不完整桶标部分覆盖且不可作标准比较。

## 3. 期间与比较规则

### 3.1 期间与策略

业务日期窗口[from,to]闭区间。事件UTC RFC3339保存，按工作区确认的IANA时区转换日历日期，禁止宿主时区隐式参与。工作区默认Asia/Shanghai，GUI首次显示确认；CLI省略时明确输出采用此默认。week_start_day ISO1–7默认1（周一），fiscal_year_start_month默认1；改动创建policy版本。

接受YYYY-MM-DD、明确YYYY/MM/DD；时间字符串须含时区或确认来源时区。01/02/26等歧义拒绝。Excel1900/1904系统按文件元数据，序列60在1900系统报invalid_excel_date；没有日期格式或映射的数字不猜日期。

日为一天；周从配置起日算7天；月自然月；财季以财年起月锚定连续3月，FY按起始年命名。完整标准期间的下一窗口为下个完整同类期间；custom或部分期间默认紧接to的同天数范围，标“等长自定义行动窗口”，不假称自然月。用户可显式改行动窗口，另入指纹。

policy生效[from,to)左闭右开，最后to=null；拒绝重叠。默认分析跨policy边界报policy_selection_required；用户显式选一版本作用于整个范围，标override并保存快照，不能自动分段混算。原历史保持旧版本。

### 3.2 基期算法

| grain | 环比 | 同期 |
| --- | --- | --- |
| day | 前1当地日 | 上一年同月同日；闰日不存在则null/date_not_in_prior_year |
| week | 全周前移7天 | 全周前移364天，固定标签“52周同期”，不称自然年同周，不切换ISO53周 |
| month | 前一完整自然月 | 上一年同月 |
| quarter | 前一财季（前3月） | 同财年配置前12月的财季 |
| custom | 紧接from前的等天数范围 | 两端减1年均存在且总天数相同，否则prior_year_range_not_comparable |

标准grain不覆盖完整桶时partial_period，不作标准比较；用户可改custom另分析。完整自然月长短不同不阻止月环比，custom不可默默截短。

兼容签名=metric semantic hash+unit/currency/tax+scope+timezone+目标grain+week_start(仅week)+fiscal_start(仅quarter)，不含policy版本、文件名或原输入粒度。签名不同标definition/unit/scope/calendar_mismatch。完整日可重聚月后比历史月，月不能拆日；保留基期run/revision与聚合来源。

两方valid且complete兼容时absolute_change=current−base；base>0才算change_rate=(current−base)/base。base=0或负数只给绝对变化，百分比null且zero_base/negative_base；不使用abs(base)。一方缺失则变化与比例均null。direction仅决定是否有利，不改变化率公式。历史比较固定base run/revision，不随基期重做漂移，只有本期重算才刷新。

## 4. 模板识别与字段映射

### 4.1 解析共性

首版运行时只读xlsx/xls/csv；PPTX是设计参考，不是运行时源，扫描时标unsupported并不外发。CSV接受UTF-8/BOM；失败要求用户选GB18030，不猜乱码。仅表头匹配做首尾/换行空白清理、全角ASCII转半角、连续空格合一；其他原文保持。exact别名匹配，不用模糊或LLM映射。

首50行搜索连续1–3行表头块：至少两个已知字段，取命中必需字段最多者；同分多个需用户选。分块sheet每块独立识别。合并表头仅左上标签向覆盖列投影；业务层级标签可在明确分组内继承，合并数值绝不复制到每个子行，空数值不forward-fill。整行合并标题/说明段不是交易行。隐藏行/列在预览标示，由选范围决定，不静默排除。

重复列用列字母区分，未知列进unmapped，不丢列。映射绑定source hash、sheet、表头signature；header变更需重新确认。catalog已知公式重算；未知公式unsupported_formula，缓存仅人工对照，不作为valid。

### 4.2 表头别名（除此之外手动映射）

| 规范字段 | exact别名 | 类型/限制 |
| --- | --- | --- |
| metric_label | 科目、项目、指标、指标目标、MP名称、MP指标 | string；指标目标混合文字不得自动抽数字 |
| source_note | 计算公式/取数来源、计算公式、取数来源、数据来源、来源说明 | string，不执行 |
| mp_label | 本月MP、MP、重点指标 | string，不凭标签猜数值 |
| unit | 单位、计量单位 | 明确单位 |
| annual_month_plan | 年度计划·当月目标、年度当月目标、年度计划当月目标、年度月度分解目标 | decimal且明确月份；年度计划单独视annual_total_plan，不能除12 |
| planned | 月度预定、本月预定、本期预定、期间预定、预定、计划值、目标值、本期目标、目标 | decimal |
| actual | 月度实绩、本月实绩、本期实绩、期间实绩、实绩、实际值、本期实际 | decimal |
| delta | 预实差异、预实差异额、差异额、绝对差异 | decimal，重算验证，不重复作为费用 |
| plan_ratio | 预定比、预定达成率、计划达成率、达成率 | ratio，重算验证 |
| annual_plan_ratio | 年度计划比、年度计划达成率 | ratio，分母须年度当月计划 |
| sales_share | 预定销售额占比、实绩销售额占比、销售额占比 | ratio，表明planned/actual，分母须对应收入 |
| period | 期间、日期、业务日期、交易日期、月份、季度 | 明确date/month/quarter，不从文件名猜 |
| document_id/line_id | 单据号、订单号、发票号、业务单号 / 行号、明细行号、行ID | string，确认复合行键 |
| scope | 业务范围、部门、货品、仓库、客户、对象 | 每列单独维度；真实值仅用户本机 |
| task_title/task_action | 任务名称、任务、事项 / 行动方案、行动步骤、具体行动、怎么做 | string；无显式分隔规范时整体1步，不按语义猜多步 |
| owner | 责任人、责任岗位、负责人、责任 | string/null，不映射账号 |
| planned_start/due_date | 开始日期、计划开始 / 时间节点、截止日期、计划完成日期 | date/null；月底/尽快原文留存待确认 |
| task_status | 状态、完成状态 | 待开始/未开始→not_started；进行中→in_progress；已完成→completed；有阻塞/阻塞→blocked；已延期只作overdue源标记 |
| progress | 进度、完成率 | 本地0–100的decimal字符串；文本50%→50，Excel百分数格式底值0.5→50；普通数字0.5仍是0.5，不猜50。最多2位小数且不越界 |
| evidence | 任务依据、依据、证据、来源 | 文本+坐标，不能升级随意文字为source fact |
| category | 方向、类别、经营方向 | 按task-lifecycle §4.2同一exact分类映射：revenue/cost/efficiency/other及列明中文别名；未知类别人工映射，不默认other |
| review_note/next_plan | 复盘说明、检视说明 / 后续计划 | string，保存原文，不能自动生成任务 |

指标行名只按catalog.label_aliases映射。未注册行须用户确认custom metric的单位、语义、方向及聚合。合计/小计/总计行和累计列先标aggregate，可作范围观察或对账，不能和明细重复求和。

### 4.3 适配器与已知结构锚点

本节锚点来自本机只提取表头/合并结构的核查，无业务行/数值。坐标用于说明真实布局形态，不是唯一匹配条件：

- accounting：sheet`核算表`且metric_label+至少一项actual/planned/annual_month_plan；已知第4行可出现B科目、C计算公式/取数来源、D MP、E年度当月目标、F月度预定、G预定销售额占比、H月度实绩、I实绩占比、J差异、K预定比、L年度计划比。跨整行合并项为section。`差异分析`可为B目标、C实绩、E状态的稀疏结构，空分隔列不丢失；状态文字不是数字。
- trend：sheet`推移表`，metric_label或A列科目锚点+至少一可解析期间列；第4行可为期间列、L实绩/预定、M预定比、N年度计划比；累计行/列单独标范围观察。时期接受YYYY-MM、YYYY年M月、YYYY-MM-DD或明确起止；`1月`须已有明确年元数据。图表sheet可能只有标题/内嵌chart，只取chart引用的已解析单元格作证据，不从图像猜值。
- task：sheet后缀`任务单检视`或`任务单计划`（允许当月/下月/本期/下期/日期前缀），另有`任务依据`；表头须task_action或metric_label，且owner/due_date/planned/actual之一。已知检视布局为类别、指标目标、MP、预定、实绩、达成率、复盘说明、后续计划；计划布局为类别、指标目标、MP、预定、行动方案、时间节点、责任、状态。源“已完成”仅为导入候选信息，不凭标题覆盖应用任务。
- 任务依据可有多块重复表头（例如不同位置起始），逐块识别并分配source_ref，标题/说明不当明细；未识别块进人工范围映射。不同块业务范围不相同时不合并。
- 一文件可命中多个sheet角色，每sheet只一个；冲突需用户选，不能按适配器顺序吞表。独立推移与核算内推移依§2去重。
- 不满足结构则generic_table：用户确认表头/数据范围、字段/单位、日期、record_kind、业务行键、coverage。部分识别保留待映射，不声称全表完成。

## 5. 空白模板和导出布局

`import-template-v1.zip`含三份空白XLSX和字段说明Markdown，无样例金额、客户、人员。统一第1行template_version=1，第2行workspace/currency/tax，第3行source_from/source_to/grain，第4行policy timezone/week_start/fiscal_start及coverage，第5行空，第6行列名，第7行起输入。元数据为空时必须运行时补齐。

| 文件/sheet | 从A列起的列布局 |
| --- | --- |
| 核算表模板.xlsx / 核算表 | 科目、单位、计算公式/取数来源、本月MP、年度计划·当月目标、月度预定、月度实绩、预实差异额、预定比、年度计划比 |
| 核算表模板.xlsx / 推移表 | 科目、单位、取数来源、期间列…、累计 |
| 核算表模板.xlsx / 差异分析 | 科目、单位、本期目标、本期实际、绝对差异、差异方向、同比基期、同比绝对差、同比变化率、环比基期、环比绝对差、环比变化率、缺失/不可比原因、证据 |
| 推移表模板.xlsx / 推移表 | 同上推移布局，重复来源不能双计 |
| 推移表模板.xlsx / 图表 | chart_id、图表类型、metric_key、from、to、grain、source_refs；空配置，不造趋势 |
| 任务单模板.xlsx / 本期任务单检视 | 类别、任务名称、指标、单位、目标值、实际值、行动方案、责任人、开始日期、截止日期、状态、进度、检视说明、任务依据 |
| 任务单模板.xlsx / 下期任务单计划 | 类别、任务名称、指标、单位、目标值、行动方案、责任人、开始日期、截止日期、资源需求、完成标准、待确认项、任务依据 |
| 任务单模板.xlsx / 任务依据 | 依据ID、指标、来源期间、目标期间、来源文件标签、工作表、单元格/行、比较类型、基期、基期值、本期值、绝对变化、变化率、待确认原因 |

元数据精确位置：A1为dsheng_template_version、B1为1；A2/B2为workspace_name/值，C2/D2为currency/值，E2/F2为tax_basis/值；A3/B3为source_from/值，C3/D3为source_to/值，E3/F3为grain/值；A4/B4为timezone/值，C4/D4为week_start_day/值，E4/F4为fiscal_year_start_month/值，G4/H4为coverage_status/值，I4/J4为empty_days_are_zero/值。导出时可继续K3/L3存target_from/值、M3/N3存target_to/值；任务目标窗口不可与来源窗口混用。无元数据的旧模板使用导入预览显式补齐，不能硬套上述行位。

可预列catalog显示行名，数值全空；配置型指标标需配置。空期间列为“期间1（填写ISO期间）”等，未填不能导成真实期间。非月grain将月度标题显示为本期；不能把日度写成月度。

业务导出仍为三份工作簿，使用上述字段。任务sheet名称固定`任务单检视`和`任务单计划`，完整期间放第3行，避免Excel31字符及重名截断；导入器接受无日期前缀同一后缀。任务多步骤按永久task_id分组展开行，任务字段重复，不合并步骤值，空日期保留空。实际导出包含task_id/step_id技术列（可隐藏但在字段说明中列出），不得拿行号作身份。

文本以=、+、-、@开头时XLSX写文本类型，CSV以单引号安全转义；真正数值（包括负数）保留数值类型，不能套用字符串防护改成别的数值。派生值写已计算字面量，不写未知公式。XLSX超过15位有效数写精确decimal文本并标number_storage=decimal_text；其余可写数值。业务JSON保持decimal字符串。

导出metadata含workspace/run/revision/as_of、policy、指标hash、source manifests、基期run/revision、过滤器和图表数据来源；不写本机绝对路径/Key。ratio/stock无“累计”语义时累计为空并注明。只有1有效桶展示单期图，缺桶不造值。

会议版固定10页JSON/Markdown投影：1期间与完整性；2目标；3核算预实；4差异及证据/待确认；5推移；6同比环比/不可比原因；7已确认追赶目标/行动；8本期任务检视；9下期任务计划；10证据与风险。缺第7页内容显示未提供，不能为补页调用模型。不生成PPTX。

## 6. 验收与变更

合成用例必须由独立Decimal/日期计算验证期望值，再接入Rust测试；UI/CLI读取同一核心结果。模板产物须空白打开→填合成值→导入→导出往返，核对布局、合并、空值、数字精度、总计/累计和三文件边界。规则已固定；企业填写口径/来源/覆盖是产品运行时输入，不是工程师未做的业务分析。缺必要输入时返回明确原因。
