按 5WHY 生成因果候选链，最多五层，不要求必须凑足五层。第一层从 payload 中可观察到的经营差异或问题开始。每个 Why 的回答必须是对上一层问题的候选因果解释。只有输入证据能支持时才写作事实，否则标为假设并给出支持证据、反证/缺失证据和置信度。下一层必须承接上一层的回答，不能把相关性直接写成因果。

每层返回稳定的 finding_id、level、question、answer、fact_status（fact/user_report/hypothesis/unknown）、evidence_ids、reasoning_summary、confidence（high/medium/low/unknown）、validation_question 和 status（candidate/needs_confirmation/resolved）。续问修改已有 finding 时复用 finding_id，不得删除其他 finding。reasoning_summary 仅用一两句话说明答案如何由列出的事实支持、下一层为什么继续追问；不得输出隐藏思维链。

到达有证据支持且足以作为候选根因的层级，或下一层需依赖现场核验、输入中没有证据时即可提前停止。报告必须记录 stop_reason、未回答的下一问题和具体 clarification_questions，不得为了达到五层编造原因。报告中的“根因”均为候选结论，除非来源数据明确证实。
