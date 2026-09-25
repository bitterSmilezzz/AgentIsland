using System.IO;
using System.Net;
using System.Text;
using System.Text.Json;

namespace AgentIsland.Core;

/// 本地 Webhook（127.0.0.1:41999，与 macOS 端同端口同协议）：
/// · POST /notify、/event：无鉴权直推事件（岛内标「外部投递」）
/// · POST /session、DELETE /session：Agent 自报生命周期，须出示令牌 X-AgentIsland-Token
///   （令牌文件 %APPDATA%\AgentIsland\report.token）。TTL 钳 15–600 秒，到期只盖章不删除。
public sealed class LocalEventServer : IDisposable
{
    private readonly ActivityEngine _engine;
    private HttpListener? _listener;
    private CancellationTokenSource? _cts;

    private sealed record SelfReport(string AgentKey, ActivityLevel Level, DateTimeOffset ExpiresAt);
    private readonly Dictionary<string, SelfReport> _reports = [];
    private readonly object _lock = new();

    public LocalEventServer(ActivityEngine engine)
    {
        _engine = engine;
    }

    public static string TokenFilePath => Path.Combine(
        Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData), "AgentIsland", "report.token");

    /// 读取或生成令牌（0600 形态的普通文件；这里不追求伪不可伪造，只验形态）
    public static string EnsureToken()
    {
        try
        {
            var dir = Path.GetDirectoryName(TokenFilePath)!;
            Directory.CreateDirectory(dir);
            if (File.Exists(TokenFilePath))
            {
                var existing = File.ReadAllText(TokenFilePath).Trim();
                if (existing.Length >= 32 && existing.All(Uri.IsHexDigit)) return existing;
            }
            var token = Guid.NewGuid().ToString("N") + Guid.NewGuid().ToString("N");
            File.WriteAllText(TokenFilePath, token);
            return token;
        }
        catch (IOException)
        {
            return Guid.NewGuid().ToString("N");
        }
    }

    public void Start()
    {
        if (_listener != null) return;
        _listener = new HttpListener();
        _listener.Prefixes.Add("http://127.0.0.1:41999/");
        try
        {
            _listener.Start();
        }
        catch (HttpListenerException)
        {
            _listener = null;
            return; // 端口被占：岛其余功能不受影响
        }
        _cts = new CancellationTokenSource();
        _ = Task.Run(() => AcceptLoopAsync(_cts.Token));
    }

    public void Stop()
    {
        _cts?.Cancel();
        try { _listener?.Stop(); _listener?.Close(); } catch { }
        _listener = null;
    }

    private async Task AcceptLoopAsync(CancellationToken ct)
    {
        while (!ct.IsCancellationRequested)
        {
            HttpListenerContext ctx;
            try { ctx = await _listener!.GetContextAsync().WaitAsync(ct); }
            catch (OperationCanceledException) { break; }
            catch (Exception) { break; }
            _ = Task.Run(() => Handle(ctx), ct);
        }
    }

    private void Handle(HttpListenerContext ctx)
    {
        try
        {
            var path = ctx.Request.Url?.AbsolutePath ?? "/";
            var method = ctx.Request.HttpMethod;
            string body;
            using (var reader = new StreamReader(ctx.Request.InputStream, Encoding.UTF8))
                body = reader.ReadToEnd();

            var (status, response) = Route(path, method, body, ctx.Request);
            var bytes = Encoding.UTF8.GetBytes(response);
            ctx.Response.StatusCode = status;
            ctx.Response.ContentType = "application/json; charset=utf-8";
            ctx.Response.ContentLength64 = bytes.Length;
            ctx.Response.OutputStream.Write(bytes);
            ctx.Response.OutputStream.Close();
        }
        catch (Exception)
        {
            try { ctx.Response.StatusCode = 500; ctx.Response.OutputStream.Close(); } catch { }
        }
    }

    private (int status, string response) Route(string path, string method, string body, HttpListenerRequest req)
    {
        // 自报生命周期：需要令牌
        if (path is "/session")
        {
            if (method == "DELETE")
            {
                var keyDel = req.QueryString["agent"] ?? "";
                lock (_lock) _reports.Remove(keyDel);
                return (200, """{"ok":true}""");
            }
            if (method == "POST")
            {
                var token = req.Headers["X-AgentIsland-Token"] ?? "";
                if (token != EnsureToken() || token.Length < 32)
                    // 没令牌的申报不丢：落回无鉴权通道，带「未采信」标记
                    return NotifyRoute(body, untrusted: true);

                try
                {
                    using var doc = JsonDocument.Parse(body);
                    var agent = doc.RootElement.TryGetProperty("agent", out var a) ? a.GetString() ?? "" : "";
                    var state = doc.RootElement.TryGetProperty("state", out var s) ? s.GetString() ?? "" : "";
                    var ttl = doc.RootElement.TryGetProperty("ttl_seconds", out var t) && t.TryGetInt32(out var n) ? n : 90;
                    ttl = Math.Clamp(ttl, 15, 600);
                    var level = state switch
                    {
                        "working" => ActivityLevel.Working,
                        "idle" => ActivityLevel.Idle,
                        "attention" => ActivityLevel.Attention,
                        "completed" => ActivityLevel.Completed,
                        _ => (ActivityLevel?)null,
                    };
                    if (agent.Length == 0 || level == null)
                        return (400, """{"error":"agent 与 state(working|idle|attention|completed) 必填"}""");
                    lock (_lock)
                    {
                        _reports[agent] = new SelfReport(agent, level.Value, DateTimeOffset.Now.AddSeconds(ttl));
                        if (_reports.Count > 200) _reports.Clear();
                    }
                    return (200, """{"ok":true,"note":"自报不参与显示，岛与 CLI 只看推断状态"}""");
                }
                catch (JsonException)
                {
                    return (400, """{"error":"invalid json"}""");
                }
            }
            return (405, """{"error":"method not allowed"}""");
        }

        if (path is "/notify" or "/event" && method == "POST")
            return NotifyRoute(body, untrusted: false);

        return (404, """{"error":"not found"}""");
    }

    private (int, string) NotifyRoute(string body, bool untrusted)
    {
        try
        {
            using var doc = JsonDocument.Parse(body);
            var root = doc.RootElement;
            var agent = root.TryGetProperty("agent", out var a) ? a.GetString() ?? "" : "";
            var eventStr = root.TryGetProperty("event", out var e) ? e.GetString() ?? "completed" : "completed";
            var message = root.TryGetProperty("message", out var m) && m.ValueKind == JsonValueKind.String ? m.GetString() : null;
            var detail = root.TryGetProperty("detail", out var d) && d.ValueKind == JsonValueKind.String ? d.GetString() : null;
            if (agent.Length == 0) return (400, """{"error":"agent 必填"}""");

            var type = eventStr switch
            {
                "attention" => AgentTaskEvent.EventType.Attention,
                "costSpike" or "cost_spike" or "alert" => AgentTaskEvent.EventType.CostSpike,
                "completed" => AgentTaskEvent.EventType.Completed,
                _ => (AgentTaskEvent.EventType?)null,
            };
            if (type == null) return (400, """{"error":"event 须为 completed|attention|costSpike"}""");

            _engine.RaiseExternalEvent(agent, type.Value, message, detail);
            var badge = untrusted ? "untrusted" : "external";
            return (200, $$"""{"ok":true,"delivered":"{{badge}}"}""");
        }
        catch (JsonException)
        {
            return (400, """{"error":"invalid json"}""");
        }
    }

    public void Dispose() => Stop();
}
