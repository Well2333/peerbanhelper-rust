# 一级规范索引（memory/guidelines）

> 本目录是**最高优先级、有约束力**的项目规范。任何改动若触及规范，必须在**同一次提交**内同步更新对应文件。
> 过程规范以 `memory/最高优先级工作守则.md` 为准；本目录承载**项目特定**的稳定约定。

## 本目录文件

| 文件 | 内容 |
|---|---|
| `01-scope-and-decisions.md` | 范围/决策**权威**：目标、四项决策、完全移除清单、下载器 trait、表集、选型 |
| `02-architecture.md` | 架构约定**权威**：不照搬上游、crate 分层、依赖抽象、流水线/调度、peer 地址 |
| `03-coding-conventions.md` | Rust 风格、错误处理、命名、序列化对齐、等价性对拍、骨架 std-only 政策 |
| `04-workflow.md` | 分支/提交/变更记录/测试政策（对接守则，补项目细节） |

## `memory/` 整体布局

- `memory/guidelines/` — 一级规范（本目录，权威、绑定性）。
- `memory/design/` — 详细设计/参考（非"简明规范"，体量较大）：
  - `roadmap.md`（路线图与施工指南：API/里程碑/验收/对拍）
  - `architecture-analysis.md`（上游系统事实分析，查源码基准）
  - `db-schema.md`（嵌入式 SQLite 表结构）
- `memory/changelog/` — 二级，每提交一条变更记录。
- `memory/test-status/` — 已测记录 / 待测报告。

> 架构设计/技术决策属仓库内部长期记忆，放 `memory/`，不放面向用户的 `docs/`。
> 冲突时：一级规范 > design/changelog；被证伪的记忆立即更正或删除。
