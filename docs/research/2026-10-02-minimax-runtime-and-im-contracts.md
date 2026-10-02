# MiniMax Code 与预设 IM 本地接入契约

核实日期：2026-10-02。凭据与真实会话内容不进入本文。

## MiniMax Code

本机桌面包为 `MiniMax Code.app` 3.1.0，`CFBundleIdentifier=com.minimax.agent.cn`；MiMo 桌面包为 `Xiaomi MiMo.app` 26.929.292248，`com.xiaomi.mimo.desktop`。使用 Info.plist 指定的 ICNS 原样转 PNG，图标来源与 SHA-256 见资产清单。桌面检测匹配包标识与主进程，排除 Electron helpers。

官方 [README](https://github.com/MiniMax-AI/minimax-code/blob/564e9166d81f87b0b767b005e4779d4697b512be/README.md) 声明 CLI 为 `mcode`、默认数据目录为 `~/.minimax`，命名 profile 使用独立目录。本轮仅接默认目录，不能以它代表所有命名 profile。

本机以 SQLite `mode=ro` 查询 `sqlite_master`、`PRAGMA table_info`，核对 `v2/sqlite/runtime-state.sqlite`：

- `local_runtime_sessions` 有 `session_id/status/updated_at_ms/archived`。
- `local_runtime_token_usage` 有 `ts/input_tokens/output_tokens/reasoning_tokens/cache_read_tokens/cache_write_tokens/cost_usd/model`。
- 不读取 `record_json/raw` 或消息表正文，不运行 checkpoint 或迁移。

官方 [pi-usage.ts](https://github.com/MiniMax-AI/minimax-code/blob/564e9166d81f87b0b767b005e4779d4697b512be/packages/local-runtime-v2/src/service/session-system/usage/pi-usage.ts) 分开记录新鲜输入、输出、推理和缓存计数；账本净用量为前三项之和，不能再加入缓存或并扫消息镜像。时间戳为毫秒。费用取账本记录，不另猜价。

状态只在新鲜窗口内采信 started；idle 必须伴随接近状态更新时间的用量才视为完成，未使用的 idle、过期 started 和归档会话不产生完成/在途信号。它是本地投影，不承诺覆盖所有框架的授权等待状态。

## IM

- [飞书官方示例](https://www.feishu.cn/content/7271149634339422210)：文本请求；签名用 `timestamp\nsecret` 作为 HMAC-SHA256 key、空消息、Base64 编码。Webhook 限飞书官方 HTTPS bot 路径，无 userinfo、query 或 fragment。签名与 Webhook 合存系统钥匙串，设置 JSON 仅存非凭据选项。
- [PushPlus 官方 API](https://www.pushplus.plus/doc/guide/api.html)：微信 channel=wechat，QQ channel=qq，纯文本 template=txt；QQ 需先完成平台绑定，群配置编码是可选 option。业务 code=200 表示请求受理，不能据此宣称最终手机送达。
- [OneBot 11](https://github.com/botuniverse/onebot-11/blob/master/api/public.md)：HTTP `/send_group_msg`，数字 group_id，text 消息段避免 CQ 码被解释；令牌放 Authorization。仅接已有机器人，不提供 QQ 登录或托管。

预设通道检查明确的 JSON 成功码；业务失败不回显平台文本，不自动重试。自定义模板保留 HTTP 状态口径。响应头与正文有界，支持 Content-Length、chunked 和连接关闭帧，不跟随重定向。
