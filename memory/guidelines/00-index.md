# 一级规范索引（memory/guidelines）

> 本目录是**最高优先级、有约束力**的项目规范。任何改动若触及规范，必须在**同一次提交**内同步更新对应文件。
> 过程规范以 `docs/最高优先级工作守则.md` 为准；本目录承载**项目特定**的稳定约定。

| 文件 | 内容 |
|---|---|
| `01-scope-and-decisions.md` | 项目范围边界 + 三项已确认决策 + 技术选型 |
| `02-architecture.md` | crate 分层、依赖抽象、流水线/调度约定、AppContext 注入 |
| `03-coding-conventions.md` | Rust 风格、错误处理、命名、等价性对拍、骨架 std-only 政策 |
| `04-workflow.md` | 分支/提交/变更记录/测试政策（对接守则，补项目细节） |

冲突时：一级规范 > 二级 changelog；被证伪的记忆立即更正或删除。
