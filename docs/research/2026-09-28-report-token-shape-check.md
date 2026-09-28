# report.token 的形态校验：Rust 侧原实现会自我否定

日期：2026-09-28　版本：v0.0.223　机器：macOS 27.0.0 arm64

## 一句话

Swift `SelfReportTokenStore` 对 `report.token` 做五件事，Rust 侧的
`webhook::ensure_token` **一件都没做**——其中令牌本身由系统时钟播种，
而令牌的全部价值就是「猜不到」。

## 两端逐条比对

Swift：`Sources/AgentIslandCore/SelfReport.swift`（`ensure` / `inspect` / `randomToken`）
Rust：`app/src-tauri/src/webhook.rs`（v0.0.222 及以前的 `ensure_token`）

| 校验 | Swift | Rust（v0.0.222 及以前） |
| :--- | :--- | :--- |
| 令牌来源 | `SecRandomCopyBytes(kSecRandomDefault, 24)` | `format!("{:x}{:x}", rand_u64(), rand_u64())` |
| 文件 0600 | `createFile` + **写完 `setAttributes` 再收一次** | 从不设权限 |
| 目录 0700 | `createDirectory(attributes: .posixPermissions: 0o700)` | `create_dir_all` 后不管 |
| 非常规文件 | `attrs[.type] == .typeRegular`，否则 `notRegular` | 不检查 |
| 属主 | `attrs[.ownerAccountID] == getuid()`，否则 `foreignOwner` | 不检查 |
| 权限位 | `mode & 0o077 == 0`，否则 `looseMode` | 不检查 |
| 缺陷分类 | 5 类 `SelfReportTokenDefect` + `AppLog.warn` | 静默重写 |

### 原 `rand_u64` 的原文

```rust
fn rand_u64() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let n = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    (n as u64).wrapping_mul(0x9E3779B97F4A7C15).rotate_left(17)
}
```

**推断（标注为推断）**：这是「黄金比例乘法散列」，不是密码学随机。
知道大致启动时刻的进程读一次 `date +%s%N`，就能把候选区间缩到很小。
我没有实测出实际碰撞或预测，但**它在结构上就不是不可预测的**——
令牌的全部用途是「谁在说话」，而这一条直接否定那个用途。

### 三个具体后果

1. **同步盘 / `chmod -R` 之后令牌就公开了。** 目录 `Application Support/AgentIsland/`
   在首启之前归用户可写；一旦令牌变成 0644，任何本机用户都能读到，
   读到就能绑一条可信自报（`/session` 的 `X-AgentIsland-Token`），而我们以为是自己生成的。
2. **符号链接会被穿过。** 原实现用 `std::fs::write`，它跟随符号链接——
   于是「别处已经有个文件」会被就地覆盖。Swift 侧特意用 `createFile`
   并有两条用例钉着「不许把新令牌写穿符号链接」（注释里说实测 `createFile`
   会先把链接本身解掉）。Rust 侧当时没有这层保护。
3. **令牌长度与 Swift 不同。** Swift 是 24 字节 → 48 个十六进制字符；
   Rust 原实现是两个 u64 的 hex → 32 个字符。所以两端生成的文件**长度就不一样**，
   而两端共用同一个路径。

## 真机证据

### 现有的那份令牌是 Swift 版建的

```sh
ls -la ~/Library/Application\ Support/AgentIsland/report.token
```

```
drwx------@ 3 <user>  staff   96 Sep 24 16:02 .
-rw-------@ 1 <user>  staff   48 Sep 24 16:02 report.token
```

> 输出里的**属主名与组名已打码**：这份文档要进公开仓库，
> 而 `ls -la` 的原文带着本机用户名。形态（`drwx------` / `-rw-------`）
> 与字节数（48）是结论所需的部分，逐字保留。

**48 字节、0600、目录 0700** —— 恰好是 Swift `randomToken()` 的长度
（`SecRandomCopyBytes` 取 24 字节 → 48 个 hex 字符），也与它的权限设置一致。
这就是「两边共用同一个令牌文件」的实际样子（ADR 0013）。

### 新逻辑跑过之后：文件未变、日志无缺陷

```sh
cp ~/Library/Application\ Support/AgentIsland/report.token /tmp/token-before.txt
# 用新构建启动
grep -c "形态不合格" /tmp/agentisland-tauri.log     # → 0
diff -q /tmp/token-before.txt ~/Library/.../report.token   # → 未变
```

**这一条是必须成立的**：从 Swift 版升到 Rust 版时令牌若被换掉，
已配置的每一个接入方都得重配，那是一次静默的破坏。

## v0.0.223 的改法

- `inspect_token`：**`symlink_metadata` 而不是 `metadata`**。
  后者会穿过符号链接，于是「链接指向一个 0600 的真文件」会被判成合格——
  而那正是要挡住的形态。
- 写入用 `OpenOptions::create_new(true)`：保证不穿过任何已存在的路径。
  删符号链接时用 `remove_file`，删的是**链接本身**。
- 权限**显式设两次**：建目录后一次、写完文件后一次
  （新建文件的权限同样受 umask 影响）。用例里把 umask 放到 `0o000` 验证这一点。
- 熵源走 `/dev/urandom`（24 字节），不引依赖。
- 缺陷分类成 5 类并 `log_line` 出来——**静默换掉是这条通道最坏的失效方式**：
  用户看到的是「昨天还好使，今天全废」，而真实原因只是有人对目录跑过 `chmod -R`。
- `ensure_token` 返回类型从 `String` 改成 `Option<String>`：
  `None` = 拿不到可信通道，`/session` 答 401 `noToken`。
  **不可退化成「拿请求里的串比对」**——那会让「服务端没令牌」变成「谁都能自报」。
- `uuid_lite` 也换成了真熵（原来复用同一个时钟播种的 `rand_u64`）。
  它只是标识符、熵源读不到时退回时钟是可接受的降级，**令牌那条不接受**——这个区别写在注释里。

## 六条用例

`sandbox + 临时目录`，不碰真实凭据文件：

| 用例 | 钉住什么 |
| :--- | :--- |
| `a_well_formed_token_passes_inspection` | 合格的一定通过（**所有反例的前提**——少了它，「全部拒了」也能让反例变绿） |
| `a_group_readable_token_is_refused` | 0644 ⇒ `LooseMode` |
| `a_symlink_is_refused_rather_than_followed` | 符号链接 ⇒ `NotRegular`，且换令牌时**不动到它指向的文件** |
| `content_and_absence_have_distinct_reasons` | 缺失 / 太短 / 非十六进制各有各的理由 |
| `a_new_token_is_owner_only_even_under_a_loose_umask` | umask 0o000 下仍是 0600 / 目录 0700 |
| `a_generated_token_is_24_bytes_of_hex` | 48 个 hex 字符（与 Swift 同长） |

## 没验证的

- **真实令牌文件上的缺陷分支（0644 / 符号链接）没有在真机上跑过**——
  那需要改动用户正在用的凭据文件。六条用例都在沙箱临时目录里。
- **这条通道的端到端鉴权**（外部进程带 token 调 `/session` 被接受、
  不带被拒 401）本轮**没有**重新验；`webhook.rs` 里原有那几条用例仍覆盖它。
- **形态不是来源**：同一个用户、同样 0600 的合法十六进制串仍可能是抢先写好的。
  本机要真做到不可伪造，得把令牌放进钥匙串，并放弃「抄进第三方配置」这件事。
  这条与 Swift 同，属已知的诚实边界。
