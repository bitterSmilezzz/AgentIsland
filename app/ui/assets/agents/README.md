# Agent identity assets

Static monochrome SVGs from [Lobe Icons](https://github.com/lobehub/lobe-icons),
revision `79b551cf26aab9ea4ac701fb807160950a5b860f`, copied without modification
from `packages/static-svg/icons/`. The upstream MIT license is preserved in
[LICENSE](LICENSE). Product names and marks remain their respective owners’ trademarks;
use here identifies monitored tools and does not imply endorsement.

`app/ui/js/agent-icons.js` is the mapping used by every Agent identity slot.
Claude, Codex, Cursor, Cline, Roo Code, OpenCode, Xiaomi MiMo, Goose, Windsurf,
Trae, Qoder, Antigravity and Hermes Agent use corresponding marks. ChatGPT uses
OpenAI’s mark; DeepSeek Harness uses DeepSeek’s mark as a vendor identity.

The other entries use locally typeset initials, not claimed official logos.
In particular, registry id `copilot` means Tencent **ima.copilot**, so the GitHub
Copilot mark must not be used for it. WorkBuddy and WorkBuddy AI have distinct initials.
Custom agents receive escaped name initials. SVGs load locally, through a CSS mask,
with theme-aware ink; status colors are separate from identity colors.
