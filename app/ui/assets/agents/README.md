# Agent identity assets

Static monochrome SVGs from [Lobe Icons](https://github.com/lobehub/lobe-icons),
revision `79b551cf26aab9ea4ac701fb807160950a5b860f`, copied without modification
from `packages/static-svg/icons/`. The upstream MIT license is preserved in
[LICENSE](LICENSE). Product names and marks remain their respective owners’ trademarks;
use here identifies monitored tools and does not imply endorsement.

`app/ui/js/agent-icons.js` is the mapping used by every Agent identity slot.
Claude, Codex, Cursor, Cline, Roo Code, OpenCode, Xiaomi MiMo, Goose, Windsurf,
Trae, Qoder, Antigravity and Hermes Agent use corresponding marks. The unified
ChatGPT / Codex entry uses the Codex mark; DeepSeek Harness uses DeepSeek’s mark
as a vendor identity. The retained OpenAI SVG is an upstream asset, not a separate Agent entry.

The remaining identities use original AgentIsland vector pictograms under the
repository MIT license (copyright 2026 bitterSmilezzz), not claimed official logos:

| File | Identity | Visual cue |
| --- | --- | --- |
| `dim.svg` | DimAgent | Spark |
| `zcode.svg` | ZCode | Lightning inside a rounded frame |
| `vscode.svg` | VS Code | Editor ribbon |
| `aider.svg` | Aider | Paired code chevrons |
| `ima.svg` | ima.copilot | Notes with a spark |
| `workbuddy.svg` | WorkBuddy | Connected collaborators |
| `workbuddyai.svg` | WorkBuddy AI | Globe |
| `continue.svg` | Continue | Continuing arrow |
| `vibeusage.svg` | Vibe Usage | Usage trend |
| `openviking.svg` | OpenViking | Viking helmet |
| `customagent.svg` | Custom or unknown Agent | Hexagon and spark |

Registry id `copilot` means Tencent **ima.copilot**, so the GitHub Copilot mark
must not be used for it. WorkBuddy variants have different pictograms. Every
built-in identity and the custom fallback uses a local SVG through a CSS mask,
with theme-aware ink; status colors remain separate from identity colors.
