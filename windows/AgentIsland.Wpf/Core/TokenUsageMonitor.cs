using System.IO;
using System.Text.Json;

namespace AgentIsland.Core;

public sealed record ModelUsage(string Model, long Tokens, double Cost);

public sealed record HourBucket(DateTime Hour, long Tokens);

public sealed record TokenReport(
    TokenUsage Usage,
    List<ModelUsage> Models24h,
    List<ModelUsage> ModelsTotal,
    List<HourBucket> Hourly30d);

/// 跨工具用量监控：只读解析会话 JSONL 的结构化 usage 字段（净消耗口径，不含缓存读取）。
/// 按文件指纹缓存增量解析，不重复读旧字节。
public sealed class TokenUsageMonitor
{
    private sealed class FileState
    {
        public long ParsedLength;
        public DateTimeOffset LastWrite;
        public long TokensTotal;
        public double CostTotal;
        public long Tokens24h;
        public double Cost24h;
        public Dictionary<string, (long tokens, double cost)> Models = [];
        public List<(DateTimeOffset ts, string model, long tokens, double cost)> Entries = [];
    }

    private readonly Dictionary<string, FileState> _states = [];
    private readonly object _lock = new();

    public TokenReport Monitor(AgentProfile profile)
    {
        long tokens24 = 0, tokensTotal = 0;
        double cost24 = 0, costTotal = 0;
        var models = new Dictionary<string, (long tokens, double cost)>();
        var cutoff = DateTimeOffset.Now.AddHours(-24);

        foreach (var root in profile.TokenRoots)
        {
            if (!Directory.Exists(root)) continue;
            foreach (var file in SafeEnumerate(root))
            {
                var (t24, c24, tTotal, cTotal, fileModels) = ParseFile(file, cutoff);
                tokens24 += t24; cost24 += c24; tokensTotal += tTotal; costTotal += cTotal;
                foreach (var (model, (tk, co)) in fileModels)
                {
                    var (pt, pc) = models.TryGetValue(model, out var cur) ? cur : (0L, 0.0);
                    models[model] = (pt + tk, pc + co);
                }
            }
        }

        var modelList = models
            .Select(kv => new ModelUsage(kv.Key, kv.Value.tokens, kv.Value.cost))
            .OrderByDescending(m => m.Tokens)
            .ToList();

        // 30 天逐小时桶（趋势图 / 24h 节律热力图）
        var hourly = new Dictionary<DateTime, long>();
        var cutoff30d = DateTimeOffset.Now.AddDays(-30);
        lock (_lock)
        {
            foreach (var state in _states.Values)
            {
                lock (state)
                {
                    foreach (var (ts, _, tokens, _) in state.Entries)
                    {
                        if (ts < cutoff30d || tokens <= 0) continue;
                        var hour = new DateTime(ts.Year, ts.Month, ts.Day, ts.Hour, 0, 0);
                        hourly[hour] = hourly.TryGetValue(hour, out var v) ? v + tokens : tokens;
                    }
                }
            }
        }
        var hourlyList = hourly.OrderBy(kv => kv.Key).Select(kv => new HourBucket(kv.Key, kv.Value)).ToList();

        return new TokenReport(
            new TokenUsage { Tokens24h = tokens24, TokensTotal = tokensTotal, Cost24h = cost24, CostTotal = costTotal },
            modelList,
            modelList,
            hourlyList);
    }

    private static IEnumerable<string> SafeEnumerate(string root)
    {
        string[] files = [];
        try { files = Directory.GetFiles(root, "*.jsonl", SearchOption.AllDirectories); }
        catch (IOException) { }
        catch (UnauthorizedAccessException) { }
        foreach (var f in files) yield return f;
    }

    private (long t24, double c24, long tTotal, double cTotal, Dictionary<string, (long, double)> models)
        ParseFile(string path, DateTimeOffset cutoff24h)
    {
        lock (_lock)
        {
            var st = GetState(path);
            try
            {
                var info = new FileInfo(path);
                if (!info.Exists) return (0, 0, st.TokensTotal, st.CostTotal, []);
                // 未变化的文件直接复用缓存
                if (info.Length == st.ParsedLength && info.LastWriteTimeUtc == st.LastWrite.UtcDateTime)
                    return Aggregate(st, cutoff24h);

                using var fs = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.ReadWrite | FileShare.Delete);
                if (fs.Length < st.ParsedLength) st.ParsedLength = 0; // 文件被截断/重写：全量重读
                fs.Seek(st.ParsedLength, SeekOrigin.Begin);
                using var reader = new StreamReader(fs);
                while (reader.ReadLine() is { } line)
                {
                    if (line.Length < 8) continue;
                    ParseLineClaudeOrCodex(line, st);
                }
                st.ParsedLength = fs.Length;
                st.LastWrite = new DateTimeOffset(info.LastWriteTimeUtc);
            }
            catch (IOException) { }
            catch (UnauthorizedAccessException) { }
            return Aggregate(st, cutoff24h);
        }
    }

    private FileState GetState(string path)
    {
        if (_states.TryGetValue(path, out var st)) return st;
        st = new FileState();
        _states[path] = st;
        // 状态表有界：太多文件时丢最旧的
        if (_states.Count > 8000)
        {
            foreach (var k in _states.OrderBy(kv => kv.Value.LastWrite).Take(1000).Select(kv => kv.Key).ToList())
                _states.Remove(k);
        }
        return st;
    }

    private static (long, double, long, double, Dictionary<string, (long, double)>) Aggregate(
        FileState st, DateTimeOffset cutoff24h)
    {
        long t24 = 0; double c24 = 0;
        var models = new Dictionary<string, (long, double)>();
        lock (st)
        {
            foreach (var (ts, model, tokens, cost) in st.Entries)
            {
                if (ts >= cutoff24h)
                {
                    t24 += tokens; c24 += cost;
                    var (pt, pc) = models.TryGetValue(model, out var cur) ? cur : (0L, 0.0);
                    models[model] = (pt + tokens, pc + cost);
                }
            }
            // 有界：只保留近 7 天条目
            var cutoff7d = DateTimeOffset.Now.AddDays(-7);
            if (st.Entries.Count > 0 && st.Entries[0].ts < cutoff7d)
                st.Entries.RemoveAll(e => e.ts < cutoff7d);
        }
        return (t24, c24, st.TokensTotal, st.CostTotal, models);
    }

    private void ParseLineClaudeOrCodex(string line, FileState st)
    {
        JsonDocument doc;
        try { doc = JsonDocument.Parse(line); } catch (JsonException) { return; }
        using var docLifetime = doc;
        var root = doc.RootElement;
        if (root.ValueKind != JsonValueKind.Object) return;

        // Claude: {"type":"assistant","timestamp":"...","message":{"model":"...","usage":{...}}}
        if (root.TryGetProperty("message", out var msg) && msg.ValueKind == JsonValueKind.Object &&
            msg.TryGetProperty("usage", out var usage) && usage.ValueKind == JsonValueKind.Object)
        {
            var (tokens, cost, model) = ReadUsage(usage,
                inputKey: "input_tokens", outputKey: "output_tokens",
                cacheReadKey: "cache_read_input_tokens", cacheWriteKey: "cache_creation_input_tokens",
                model: msg.TryGetProperty("model", out var m) && m.ValueKind == JsonValueKind.String ? m.GetString() : null,
                isClaude: true);
            if (tokens > 0)
            {
                var ts = root.TryGetProperty("timestamp", out var tsEl) && tsEl.ValueKind == JsonValueKind.String &&
                         DateTimeOffset.TryParse(tsEl.GetString(), out var parsed)
                    ? parsed : DateTimeOffset.Now;
                lock (st)
                {
                    st.TokensTotal += tokens;
                    st.CostTotal += cost;
                    st.Entries.Add((ts, model, tokens, cost));
                }
            }
            return;
        }

        // Codex: {"type":"event_msg","timestamp":"...","payload":{"type":"token_count","info":{"total_token_usage":{...}}}}
        if (root.TryGetProperty("payload", out var pl) && pl.ValueKind == JsonValueKind.Object &&
            pl.TryGetProperty("type", out var pt) && pt.GetString() == "token_count" &&
            pl.TryGetProperty("info", out var info2) && info2.ValueKind == JsonValueKind.Object &&
            info2.TryGetProperty("last_token_usage", out var lu) && lu.ValueKind == JsonValueKind.Object)
        {
            var (tokens, cost, model) = ReadUsage(lu,
                inputKey: "input_tokens", outputKey: "output_tokens",
                cacheReadKey: "cached_input_tokens", cacheWriteKey: null,
                model: info2.TryGetProperty("model_context_window", out _) ? "gpt-5" : "gpt-5",
                isClaude: false);
            if (tokens > 0)
            {
                var ts = root.TryGetProperty("timestamp", out var tsEl) && tsEl.ValueKind == JsonValueKind.String &&
                         DateTimeOffset.TryParse(tsEl.GetString(), out var parsed)
                    ? parsed : DateTimeOffset.Now;
                lock (st)
                {
                    st.TokensTotal += tokens;
                    st.CostTotal += cost;
                    st.Entries.Add((ts, model, tokens, cost));
                }
            }
        }
    }

    private static (long tokens, double cost, string model) ReadUsage(
        JsonElement u, string inputKey, string outputKey, string? cacheReadKey, string? cacheWriteKey,
        string? model, bool isClaude)
    {
        long Get(string k) =>
            u.TryGetProperty(k, out var v) && v.TryGetInt64(out var n) ? n : 0;

        var input = Get(inputKey);
        var output = Get(outputKey);
        var cacheRead = cacheReadKey != null ? Get(cacheReadKey) : 0;
        var cacheWrite = cacheWriteKey != null ? Get(cacheWriteKey) : 0;
        // 净消耗：不含缓存读取
        var net = input + output + cacheWrite;
        var name = ShortModelName(model);
        var (inPrice, outPrice) = PriceTable.Lookup(name, isClaude);
        var cost = (input + cacheWrite) * inPrice / 1_000_000.0
                   + output * outPrice / 1_000_000.0
                   + cacheRead * inPrice * 0.1 / 1_000_000.0;
        return (net, cost, name);
    }

    private static string ShortModelName(string? model)
    {
        if (string.IsNullOrEmpty(model)) return "unknown";
        var m = model;
        var slash = m.LastIndexOf('/');
        if (slash >= 0) m = m[(slash + 1)..];
        return m;
    }
}

public static class PriceTable
{
    /// ($/M input, $/M output)
    public static (double inPrice, double outPrice) Lookup(string model, bool isClaude)
    {
        if (model.StartsWith("opus", StringComparison.OrdinalIgnoreCase)) return (15, 75);
        if (model.StartsWith("sonnet", StringComparison.OrdinalIgnoreCase)) return (3, 15);
        if (model.StartsWith("haiku", StringComparison.OrdinalIgnoreCase)) return (1, 5);
        if (model.Contains("opus", StringComparison.OrdinalIgnoreCase)) return (15, 75);
        if (model.Contains("sonnet", StringComparison.OrdinalIgnoreCase)) return (3, 15);
        if (model.Contains("haiku", StringComparison.OrdinalIgnoreCase)) return (1, 5);
        if (!isClaude) return (1.25, 10); // GPT 系近似
        return (3, 15);
    }
}
