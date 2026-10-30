使用 5W 界定问题和经营情境，不将它写成 5WHY 因果链。分别回答 What（发生了什么/差异是什么）、Why（现有资料如何说明其重要性或可能解释；不得据此宣称已找到根因）、Where（涉及的业务范围/环节）、When（期间或发生时间）、Who（输入中明确涉及的岗位/对象）。

每个 W 返回稳定的 finding_id、dimension、question、answer、fact_status、evidence_ids、confidence、missing_information、clarification_questions 和 status（candidate/needs_confirmation/resolved）。续问修改已有 finding 时复用 finding_id，不得删除其他 finding。资料没有明确说明时answer 写字符串 unknown 并提出要补充的具体信息，不根据角色常识猜测负责人或人物。最终 summary 描述已界定的问题和仍待确认的背景。
