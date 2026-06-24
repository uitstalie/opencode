# TODO

## Edit Undo Phase 2
- [x] inline `undo` 参数回归：edit/write 工具加回 `undo` 参数，用 `Effect.fn` + 单 return 路径解决 TS 双分支推断
- [x] 连续多步撤回链：undo_edit 执行后再记录 undoHash，支持 undo → undo → undo
- [x] undo-blobs GC：过期 blob 清理策略（`~/.local/share/opencode/undo-blobs/`）
- [x] 测试：edit undo 参数、undo 还原、undo 链式撤回 4 个用例

## 高优先级
- [ ] nudge 内容动态化：从 constraint 规则自动生成 nudge，替代 `request.ts` 硬编码

## 中优先级
- [ ] 清理 `doc/` 设计文档中标记的 TODO/待接入点

## 测试
- [ ] **Permission Scope 测试用例**（`doc/permission-scope-design.md` Phase 4）：为 scope 匹配逻辑新增测试用例
- [ ] **预存失败修复**：core 3 个失败 + opencode 1 个超时 + 4 typecheck 错误

## 低优先级
- [ ] Context Epoch 替换触发时机验证（agent/model 切换）
- [ ] Skill 热加载 / filesystem watch 失效机制
- [ ] `tool_output` 结构化结果溢出处理
