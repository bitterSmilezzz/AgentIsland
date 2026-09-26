# macOS 上把 TLS 接进 Rust 外发：四个非显然的坑

2026-09-27 核实。为「外发能出 https」这件事选型并落地时踩到的四条，每条都有可复跑的取证方式。
结论是：**用 `native-tls`（系统 TLS 栈）可以，但自签证书测试要绕道系统 `openssl` 生成的 RSA 证书。**

## 为什么选 `native-tls` 而不是 `rustls`

- macOS 上它就是 Security.framework，与 Swift 侧 URLSession / Network.framework **同一套信任栈**：
  系统钥匙串里用户装的企业根证书、代理设置，行为与 Swift 那侧一致。
- 它还带进 `security-framework`（本仓下一轮钥匙串正好要用同一个 crate），一个依赖族解决两件事。
- `rustls` 要多带一个密码学后端（ring / aws-lc-rs），且**不读系统信任栈**——得多引一份 webpki-roots，
  与「系统怎么信，我就怎么信」这条口径分叉。
- 代价（写在代码注释里）：`native-tls` 的错误文本来自系统（如 "The extended key usage is not valid."），
  不如 rustls 的结构化错误好读。

## 坑 ①：macOS 的 `Identity::from_pkcs8` 只吃 **RSA**

`native_tls::Identity::from_pkcs8` 在 macOS 上走 `SecItemImport`，喂 **EC（P-256）** 的 PKCS#8 会失败：

```
cert_pem, key_pem → Err { code: -25257, message: "Unknown format in import." }
key_pem,  cert_pem → Err { code: -50,   ... }
cert_der, key_der  → Err { code: -50,   ... }
key_der,  cert_der → Err { code: -50,   ... }
```

四种组合都试过（PEM/DER 两种编码、两种顺序），EC 一律不行。native-tls **自己的测试**用的是
`rsa_to_pkcs8(...)`——即 RSA；把同一张证书换成 RSA 后立刻成功。rcgen 只生成 EC（除非外供密钥），
所以本地 TLS 服务器这一步要绕道：

```sh
/usr/bin/openssl req -x509 -newkey rsa:2048 -nodes -keyout key.pem -out cert.pem \
  -days 1 -subj "/CN=localhost" -addext "subjectAltName=DNS:localhost" \
  -addext "extendedKeyUsage=serverAuth"
```

（本机 LibreSSL 3.3.6 支持 `-addext`。证书只写在临时目录，不进仓库。）

## 坑 ②：参数顺序与文档签名**相反**

`native_tls` 的公开签名是 `from_pkcs8(pkcs8, cert)`，但 macOS 实现里的形参是
`from_pkcs8(pem, key)`——**第一个是证书、第二个是私钥**，且它会检查第二个参数是不是 PKCS#8 私钥（PEM 头里含 `BEGIN PRIVATE KEY`）。
按文档顺序调会得到 `-50`。

## 坑 ③：自签证书必须带 `extendedKeyUsage=serverAuth`

少了它，客户端握手报：

```
TLS 握手失败：The extended key usage is not valid.
```

LibreSSL 的 `req -x509` **不会**自动加这条扩展（坑 ① 的命令里那行 `-addext` 就是为它加的）。

## 坑 ④：双栈主机要逐个地址试

`localhost` 在本机解析成 `[::1]:port` 与 `127.0.0.1:port`（顺序不定），而测试服务器只监听 IPv4。
只连解析出的第一个地址会得到 `Connection refused (os error 61)`。
生产同样受影响：只监听一族的服务在双栈主机上会连不上。修法是**逐个地址 `connect_timeout`**，
与 Swift 侧 URLSession 的 happy-eyeballs 是最小等价物（不是完整实现——没有并行与延迟竞速）。

## 落地后的证据

`transport.rs` 里两条用例（都在本机离线跑）：

- `https_really_connects_end_to_end_against_a_local_certificate`：本地 TLS 服务器 + 自签证书，
  客户端注入一份信任该证书的连接器 → 断言 `Delivered`，并逐字核对服务器收到的
  `POST /island HTTP/1.1`、`X-Title` 与正文（**证明请求真的过了 TLS**，不只是断言返回枚举）。
- `https_against_an_untrusted_certificate_fails_loudly`：同一个服务器 + **默认**连接器 →
  必须失败并说清是 TLS 握手问题。**静默降级成明文是不可接受的**（反向验证：把 https 分支
  改成直接返回明文流，这条用例立刻红）。

## 仍未验证的

- **真实公网证书链**没有在本机跑过（用例都是自签证书 + 注入连接器）。系统信任栈这条路径
  依赖 Security.framework 自身，未取实样。
- **SMTP over 465**（隐式 TLS）还没接：`HttpTransport` 对 SMTP 如实报「SMTP 会话未接入」。
  它的会话逻辑与 Socket 分层是仿 Swift `SMTPSessionIO` 的设计，下一轮做。
