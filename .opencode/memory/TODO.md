# TODO

## 高优先级
- [x] 验证完整 8-section 渲染管线端到端效果（启动 opencode 检查实际 system prompt 输出）
- [x] 升级 project-onboarding skill：适配 memory V2 + `.opencode/memory/` 检查
- [ ] TUI status bar 颜色编码：context%（绿/黄/红）和 cache rate（红/黄/绿），使用 theme.success/warning/error，拆分 `<span>` 分别着色

## 中优先级
- [ ] nudge 内容动态化：从 constraint 规则自动生成 nudge，替代 `request.ts:71` 硬编码
- [ ] Memory V2 后台 LLM 提取接入（daemon fiber → LLM 调用链）
- [ ] `OPENCODE_DISABLE_STRUCTURED_PROMPT` flag 上线后监控回退频率
- [ ] 清理 `doc/` 设计稿中的 TODO 标记点

## 低优先级
- [ ] Context Epoch 替换触发时机验证（agent 切换、model 切换）
- [ ] Skill 热加载 / filesystem watch 失效机制
- [ ] `tool_output` 结构化结果溢出处理
