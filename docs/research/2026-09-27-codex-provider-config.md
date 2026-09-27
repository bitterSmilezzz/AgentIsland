# Codex 配置档位：一手核实（2026-09-27）

Phase 2（Codex 配置档位）开工前必须先核实「写什么进 `~/.codex/config.toml`」。
本文只记**核实到的东西**与**取证方式**；推断会标出来。

## 取证方式

```sh
# ① 本机实样：只打印键路径与允许列表里的非密值（值含敏感词的键一律省略）
python3 /tmp/inspect_codex.py "$HOME/.codex/config.toml"
# ② 一手文档：OpenAI codex 仓库的 config 文档（提交 3f40fbc，docs/config.md 的原文）
curl -s https://raw.githubusercontent.com/openai/codex/3f40fbc0a886ef54f494a1ff5963971bc42c036a/docs/config.md
```

## 一、本机实样（2026-09-27）

`~/.codex/config.toml` 段落：`[desktop]`、`[marketplaces.*]`、`[plugins.*]`、`[mcp_servers.codegraph]`；
根键有 `model`、`model_reasoning_effort`、`notify`、`service_tier`。

**没有 `[model_providers]` 段。** 这条与 `docs/workbench/01-current-state.md` 里
2026-09-25 那次实扫的记录（「含 `[model_providers]`」）不一致——那份快照已经过期。
本仓的规矩是「过期的限制比没有限制更误导」，所以 01 那份已按本次实样更正。

结论：本机**没有**活的 `[model_providers.*]` 例子可抄，档位写入形状只能以文档为准（下一节）。

## 二、文档原文（OpenAI codex `docs/config.md`，提交 `3f40fbc`）

档位写入要用到的字段，逐条摘原文：

- **`model`**：`The model that Codex should use.`（顶层键，默认 `gpt-5`）
- **`model_provider`**：`Identifies which provider to use from the model_providers map. Defaults to "openai".`
- **`[model_providers.<id>]`** 是 `map where the key is the value to use with model_provider to select the corresponding provider`。
  它下面我们需要的四个键：
  - `name`：`Name of the provider that will be displayed in the Codex UI.`
  - `base_url`：`The path /chat/completions will be amended to this URL…`
  - `env_key`：`If env_key is set, identifies an environment variable that must be set when using Codex with this provider.`
    ⇒ **密钥不进配置文件，只写变量名**。这正是本项目「不存储任何凭据」能成立的地方。
  - `wire_api`：`Valid values for wire_api are "chat" and "responses". Defaults to "chat" if omitted.`
  - （另有 `query_params`、`http_headers`、`env_http_headers`、`request_max_retries`、
    `stream_max_retries`、`stream_idle_timeout_ms`——我们**不写**这些，但**绝不删**用户已写的：见第四节。）
- **`[profiles.<name>]` + 顶层 `profile`**：`A profile is a collection of configuration values that can be set together.`
  并给了 `profile = "o3"` 的写法与 `--profile` 的覆盖关系。

**这条是本轮最重要的发现**：Codex 自己就有档位层（`[profiles.*]` + `profile`），
不是只有我们才能在它外面叠一层。所以「切换档位」**不需要**每次重写整份顶层配置——
只需要改一个选择键。这决定了第四节的写法。

## 三、`toml_edit` 的一条库行为（我们依赖它，因此为它写了用例）

TOML 的语义是「表一旦开始，后面的键就属于那张表」。所以给一个**已有表**的文件新增**根键**时，
如果库把它排在表后面，Codex 读到的就不是我们写的那个键——而且**不报错**。

实测（`provider::ordering_tests::a_new_root_key_lands_before_the_first_table`）：
`toml_edit` 把新根键插在**第一张表之前**，语义正确。
这条是**库行为**，升级版本可能改变，所以留成回归用例而不是注释。

## 四、据此定的写入形状

- 只动三处：顶层 `model`、顶层 `model_provider`、以及 `[model_providers.<我们的 id>]` 表。
- 更新已有 provider 表时**只写我们认识的四个键**，用户自己加的重试/查询参数一律保留
  （有专门用例 `applying_twice_keeps_the_users_extra_provider_keys`）。
- 写入走 `toml_edit`（保留注释、键顺序、缩进）而不是「序列化整个结构」——
  用户的 `#` 注释与数组缩进被冲掉是最容易被立刻察觉的破坏。
- **先备份、再原子写**：备份写不进去时配置必须一个字节都不动
  （用例 `a_failed_backup_leaves_the_config_untouched`）。

## 五、能力边界（写进界面文案，也写在这里）

- 只切换本机已有的 Codex 档位（同厂商多账号、模型与 provider 选择）。
- **不含跨厂商模型能力**：切了档不等于别家的模型就能用（Magpie 调研的直接结论）。
- API key 不由本应用保存：档位只记 `env_key` 这个**变量名**，值由用户自己放进环境变量。
- 代码里 `PROVIDER_LIMITATIONS` 就是这段；任何一次切换的结果都带它，
  界面上少写一次就没有第二道防线。

## 六、本轮**没有**做的

- 真实写入 `~/.codex/config.toml` 的端到端验证：单元测试都在沙箱里跑，
  没有对用户的真实配置文件做过一次切换（那属于用户的操作）。
- 其他工具（Claude Code 等）：Phase 2 只做 Codex，别的要各自核实文件形状后单独立项。
