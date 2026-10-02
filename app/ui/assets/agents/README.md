# Agent identity assets

Every identity slot uses `app/ui/js/agent-icons.js`. Assets are local; no browser
launch, runtime CDN, network request or icon font is required.

## Verified product artwork

Verification date: 2026-10-02. File-level source, version, processing and SHA-256
are recorded in [official-sources.json](official-sources.json). The nine application
icons were resolved from each installed bundle's Info.plist CFBundleIconFile,
with bundle identifier checked against the monitored product. ICNS/ICO conversion
only changes format and resolution; artwork and colors are preserved.

| Asset | Verified source |
| --- | --- |
| `chatgpt.png` | ChatGPT / Codex 26.930.21537, `com.openai.codex`, current bundle icon |
| `dim.png` | DimAgent 0.9.51, `com.dimcode.app` |
| `zcode.png` | ZCode 3.14.4, `dev.zcode.app` |
| `qoder.png` | Qoder 0.4.3, `com.qoder.app` |
| `vibeusage.png` | Vibe Usage 0.7.0, `ai.vibecafe.vibe-usage` |
| `workbuddy.png` | WorkBuddy 5.6.2, `com.tencent.workbuddy.mac` |
| `workbuddyai.png` | WorkBuddy AI 5.6.2, `com.workbuddy.workbuddy-ai` |
| `ima.svg` | [ima official site](https://ima.qq.com/), declared favicon |
| `trae.png` | [TRAE official site](https://www.trae.ai/), declared favicon |
| `mimodesktop.png` | Xiaomi MiMo 26.929.292248, `com.xiaomi.mimo.desktop` |
| `minimaxcode.png` | MiniMax Code 3.1.0, `com.minimax.agent.cn` |
| `openviking.svg` | [OpenViking official repository](https://github.com/volcengine/OpenViking), `docs/images/favicon.svg` |
| `dsh.svg` | [DeepSeek Harness official repository](https://github.com/deepseek-ai/deepseek-harness), `apps/web/public/favicon.svg` |

Both WorkBuddy variants have identical ICNS hashes in these installed releases;
their names distinguish them. `mimocode` identifies Xiaomi MiMo, not MiniMax Code.
`copilot` identifies Tencent ima, not GitHub Copilot. Product artwork is displayed
as an image, retaining its original colors rather than being recolored through a mask.
Only the black transparent ima and DSH marks receive a light backing; full-color app icons preserve their transparent padding without a white tile.

Product artwork and trademarks remain their respective owners' property; the
project MIT license does not relicense them. Use identifies monitored products
and does not imply endorsement. Upstream DeepSeek Harness MIT and OpenViking
AGPL texts are preserved in [LICENSE.DeepSeekHarness](LICENSE.DeepSeekHarness)
and [LICENSE.OpenViking](LICENSE.OpenViking). DeepSeek's
[brand guidelines](https://github.com/deepseek-ai/deepseek-harness/blob/master/BRAND_GUIDELINES.md)
are respected by using its mark only to identify the monitored product.

## Lobe Icons artwork

The remaining vendor SVGs were copied without modification from
[Lobe Icons](https://github.com/lobehub/lobe-icons), revision
`79b551cf26aab9ea4ac701fb807160950a5b860f`, `packages/static-svg/icons/`.
Their upstream MIT license is preserved in [LICENSE](LICENSE). The unified
ChatGPT / Codex entry uses the current installed application icon above. Retained unused vendor SVGs do not
create additional Agent entries.

## Original fallback artwork

`vscode.svg`, `aider.svg`, `continue.svg` and `customagent.svg` are original
AgentIsland identification pictograms under the repository MIT license
(copyright 2026 bitterSmilezzz), not claimed official logos. The custom SVG is
used only for unknown/custom identities. All status colors remain separate.
