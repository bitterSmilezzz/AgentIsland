# Triage Labels

The skills speak in terms of five canonical triage roles. This file maps those roles to the actual label strings used in this repo's issue tracker.

| Label in mattpocock/skills | Label in our tracker | Meaning                                  |
| -------------------------- | -------------------- | ---------------------------------------- |
| `needs-triage`             | `needs-triage`       | Maintainer needs to evaluate this issue  |
| `needs-info`               | `needs-info`         | Waiting on reporter for more information |
| `ready-for-agent`          | `ready-for-agent`    | Fully specified, ready for an AFK agent  |
| `ready-for-human`          | `ready-for-human`    | Requires human implementation            |
| `wontfix`                  | `wontfix`            | Will not be actioned                     |

When a skill mentions a role (e.g. "apply the AFK-ready triage label"), use the corresponding label string from this table.

Edit the right-hand column to match whatever vocabulary you actually use.

## ⚠️ 本仓库现状：GH_TOKEN 无标签写权限

`gh label create` / `gh issue edit --add-label` 会 403（fine-grained PAT 未授予 labels 写权限）。因此 triage 状态用**标题前缀**表达：`[needs-triage]` / `[needs-info]` / `[ready-for-agent]` / `[ready-for-human]` / `[wontfix]`。若未来 token 权限放开，删掉本节并改回真标签。
