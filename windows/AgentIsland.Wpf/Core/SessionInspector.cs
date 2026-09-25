using System.IO;
using System.Text;
using System.Text.Json;

namespace AgentIsland.Core;

/// 会话尾部强语义解析（genericTail 方言族：Claude / Codex / Cline JSONL）。
/// 只读尾部有界字节，不保留对话正文；解析失败要给出健康结论而不是谎报「待机」。
public sealed class SessionInspector
{
    private const int TailBytes = 96 * 1024;

    public AgentSessionProbe Probe(AgentProfile profile, FileActivityResult fileActivity)
    {
        var latest = fileActivity.LatestFile;
        if (latest == null || !File.Exists(latest))
            return new AgentSessionProbe(null);

        try
        {
            var lines = ReadTailLines(latest);
            if (lines.Count == 0) return new AgentSessionProbe(null);

            return profile.Id switch
            {
                "claude" => ProbeClaude(lines, latest),
                "codex" => ProbeCodex(lines, latest),
                "cline" or "roo" => ProbeCline(lines, latest),
                _ => new AgentSessionProbe(null),
            };
        }
        catch (IOException)
        {
            return new AgentSessionProbe(null,
                new SessionProbeHealth(SessionProbeFailure.UnreadableFile, latest, DateTimeOffset.Now));
        }
        catch (UnauthorizedAccessException)
        {
            return new AgentSessionProbe(null,
                new SessionProbeHealth(SessionProbeFailure.UnreadableFile, latest, DateTimeOffset.Now));
        }
    }

    // MARK: Claude Code JSONL（~/.claude/projects/<slug>/<session>.jsonl）

    private static AgentSessionProbe ProbeClaude(List<string> lines, string path)
    {
        // 自尾向前找最近一条有内容的 assistant 条目与它之后是否已有 tool_result
        string? pendingToolId = null;
        string? pendingToolName = null;
        string? pendingToolInput = null;
        var sidechains = 0;
        var endedWithAssistantText = false;
        var lastText = "";

        for (var i = lines.Count - 1; i >= 0; i--)
        {
            var line = lines[i];
            if (line.Length < 8) continue;
            JsonDocument doc;
            try { doc = JsonDocument.Parse(line); }
            catch (JsonException) { continue; }
            using var _ = doc;
            var root = doc.RootElement;
            if (root.ValueKind != JsonValueKind.Object) continue;

            var type = root.TryGetProperty("type", out var t) ? t.GetString() : null;
            if (root.TryGetProperty("isSidechain", out var sc) && sc.ValueKind == JsonValueKind.True)
                sidechains++;

            if (type == "assistant" && root.TryGetProperty("message", out var msg) &&
                msg.ValueKind == JsonValueKind.Object)
            {
                if (msg.TryGetProperty("content", out var content) && content.ValueKind == JsonValueKind.Array)
                {
                    JsonElement? toolUse = null;
                    var hasText = false;
                    foreach (var item in content.EnumerateArray())
                    {
                        var it = item.TryGetProperty("type", out var itType) ? itType.GetString() : null;
                        if (it == "tool_use") toolUse = item;
                        else if (it == "text") hasText = true;
                    }
                    if (toolUse is { } tu)
                    {
                        pendingToolId = tu.TryGetProperty("id", out var idEl) ? idEl.GetString() : null;
                        pendingToolName = tu.TryGetProperty("name", out var nameEl) ? nameEl.GetString() : null;
                        pendingToolInput = tu.TryGetProperty("input", out var inEl) ? inEl.GetRawText() : null;
                        endedWithAssistantText = false;
                        break; // 这是最新的在途工具调用，向前不再看
                    }
                    if (hasText)
                    {
                        endedWithAssistantText = true;
                        lastText = ExtractAssistantText(content);
                        break;
                    }
                }
            }
            else if (type == "user" && pendingToolId == null)
            {
                // user 条目（可能是 tool_result）：保持向前扫
            }
        }

        // pendingToolId 之后若无对应 tool_result（尾部再无该 id 的 tool_result）→ 在途
        // 注意上面 break 时 pendingToolId 是尾部最新的 tool_use；检查它之后是否有关闭
        var closed = pendingToolId != null && TailHasToolResult(lines, pendingToolId);

        string? action = null;
        if (pendingToolId != null)
            action = DescribeClaudeTool(pendingToolName, pendingToolInput);

        if (pendingToolId != null && !closed)
        {
            if (pendingToolName is "AskUserQuestion" or "ExitPlanMode")
            {
                var question = ClaudeQuestionText(pendingToolInput) ?? "等待你确认";
                return new AgentSessionProbe(
                    new AgentSessionSignal.Attention(new AgentAttentionRequest(Fingerprint(path, pendingToolId), question)),
                    SubagentCount: sidechains);
            }
            return new AgentSessionProbe(
                new AgentSessionSignal.Active(Fingerprint(path, pendingToolId), action),
                SubagentCount: sidechains);
        }

        if (endedWithAssistantText)
        {
            // 本轮明确结束：末条 assistant 纯文本 = 回答收尾
            var fp = Fingerprint(path, lastText.Length > 64 ? lastText[..64] : lastText);
            return new AgentSessionProbe(new AgentSessionSignal.Completed(fp), SubagentCount: sidechains);
        }

        return new AgentSessionProbe(null, SubagentCount: sidechains);
    }

    private static bool TailHasToolResult(List<string> lines, string toolUseId)
    {
        // 尾部最新的 tool_use 之后必须紧跟它的 tool_result；倒序第一遍已经 break，
        // 这里顺序扫尾部条目找 tool_use_id 匹配（尾部 96KB 足够覆盖）
        var foundToolUse = false;
        for (var i = lines.Count - 1; i >= 0; i--)
        {
            var line = lines[i];
            if (!line.Contains(toolUseId)) continue;
            JsonDocument doc;
            try { doc = JsonDocument.Parse(line); } catch (JsonException) { continue; }
            using var _ = doc;
            var root = doc.RootElement;
            if (root.ValueKind != JsonValueKind.Object) continue;
            if (!root.TryGetProperty("message", out var msg) || msg.ValueKind != JsonValueKind.Object) continue;
            if (!msg.TryGetProperty("content", out var content) || content.ValueKind != JsonValueKind.Array) continue;
            foreach (var item in content.EnumerateArray())
            {
                if (item.ValueKind != JsonValueKind.Object) continue;
                var it = item.TryGetProperty("type", out var itType) ? itType.GetString() : null;
                if (it == "tool_use" && item.TryGetProperty("id", out var idEl) && idEl.GetString() == toolUseId)
                    foundToolUse = true;
                else if (it == "tool_result" && item.TryGetProperty("tool_use_id", out var tidEl) && tidEl.GetString() == toolUseId)
                    return foundToolUse; // 先有 use 后有 result（倒序里 result 先出现）
            }
        }
        return false;
    }

    private static string? DescribeClaudeTool(string? name, string? inputJson)
    {
        if (name == null) return null;
        try
        {
            using var doc = JsonDocument.Parse(inputJson ?? "{}");
            var root = doc.RootElement;
            string? Get(string key) =>
                root.TryGetProperty(key, out var v) && v.ValueKind == JsonValueKind.String ? v.GetString() : null;

            return name switch
            {
                "Bash" or "BashOutput" or "KillShell" => Get("command") is { } c ? $"运行: {OneLine(c)}" : $"运行: {name}",
                "Edit" or "Write" or "NotebookEdit" or "MultiEdit" => Get("file_path") is { } f ? $"正在修改: {f}" : $"正在修改: {name}",
                "Read" => Get("file_path") is { } r ? $"正在读取: {r}" : "正在读取",
                "Grep" or "Glob" => (Get("pattern") ?? Get("query")) is { } p ? $"正在搜索: {p}" : "正在搜索",
                "Task" or "Agent" => Get("description") is { } d ? $"子任务: {d}" : "运行子任务",
                "WebFetch" or "WebSearch" => (Get("query") ?? Get("url")) is { } q ? $"正在检索: {OneLine(q)}" : "正在检索",
                "TodoWrite" => "正在更新任务清单",
                _ => $"正在执行: {name}",
            };
        }
        catch (JsonException)
        {
            return $"正在执行: {name}";
        }
    }

    private static string? ClaudeQuestionText(string? inputJson)
    {
        try
        {
            using var doc = JsonDocument.Parse(inputJson ?? "{}");
            var root = doc.RootElement;
            if (root.TryGetProperty("questions", out var qs) && qs.ValueKind == JsonValueKind.Array)
            {
                foreach (var q in qs.EnumerateArray())
                    if (q.TryGetProperty("question", out var qt) && qt.ValueKind == JsonValueKind.String)
                        return qt.GetString();
            }
            if (root.TryGetProperty("message", out var m) && m.ValueKind == JsonValueKind.String) return m.GetString();
        }
        catch (JsonException) { }
        return null;
    }

    private static string ExtractAssistantText(JsonElement content)
    {
        var sb = new StringBuilder();
        foreach (var item in content.EnumerateArray())
        {
            if (item.ValueKind == JsonValueKind.Object &&
                item.TryGetProperty("type", out var t) && t.GetString() == "text" &&
                item.TryGetProperty("text", out var txt) && txt.ValueKind == JsonValueKind.String)
                sb.Append(txt.GetString());
        }
        return sb.ToString();
    }

    // MARK: Codex rollout JSONL（~/.codex/sessions/…/rollout-*.jsonl）

    private static AgentSessionProbe ProbeCodex(List<string> lines, string path)
    {
        string? action = null;
        string? lastMsg = null;
        var pending = false;

        for (var i = lines.Count - 1; i >= 0; i--)
        {
            var line = lines[i];
            if (line.Length < 8) continue;
            JsonDocument doc;
            try { doc = JsonDocument.Parse(line); } catch (JsonException) { continue; }
            using var _ = doc;
            var root = doc.RootElement;
            if (root.ValueKind != JsonValueKind.Object) continue;
            var type = root.TryGetProperty("type", out var t) ? t.GetString() : null;
            if (type == "response_item" && root.TryGetProperty("payload", out var pl) && pl.ValueKind == JsonValueKind.Object)
            {
                var pt = pl.TryGetProperty("type", out var ptEl) ? ptEl.GetString() : null;
                if (pt == "function_call")
                {
                    var name = pl.TryGetProperty("name", out var n) ? n.GetString() : "command";
                    var args = pl.TryGetProperty("arguments", out var a) ? a.GetString() : null;
                    pending = true;
                    action = $"运行: {OneLine((name ?? "command") + " " + (args ?? ""))}";
                    break;
                }
                if (pt == "message")
                {
                    lastMsg = pl.TryGetProperty("content", out var c) ? OneLine(c.GetRawText()) : null;
                    break;
                }
            }
            if (type == "event_msg" && root.TryGetProperty("payload", out var ev) &&
                ev.ValueKind == JsonValueKind.Object &&
                ev.TryGetProperty("type", out var evt) && evt.GetString() == "agent_message")
            {
                lastMsg = ev.TryGetProperty("message", out var m) && m.ValueKind == JsonValueKind.String ? m.GetString() : null;
                break;
            }
        }

        if (pending)
            return new AgentSessionProbe(new AgentSessionSignal.Active(Fingerprint(path, action ?? "codex"), action));

        if (lastMsg != null)
            return new AgentSessionProbe(new AgentSessionSignal.Completed(Fingerprint(path, lastMsg.Length > 64 ? lastMsg[..64] : lastMsg)));

        return new AgentSessionProbe(null);
    }

    // MARK: Cline / Roo ui_messages.json（JSON 数组投影）

    private static AgentSessionProbe ProbeCline(List<string> lines, string path)
    {
        var joined = string.Join("", lines);
        try
        {
            using var doc = JsonDocument.Parse(joined);
            if (doc.RootElement.ValueKind != JsonValueKind.Array) return new AgentSessionProbe(null);
            JsonElement? last = null;
            foreach (var item in doc.RootElement.EnumerateArray()) last = item;
            if (last is not { } el || el.ValueKind != JsonValueKind.Object) return new AgentSessionProbe(null);

            var ask = el.TryGetProperty("ask", out var askEl) ? askEl.GetString() : null;
            var say = el.TryGetProperty("say", out var sayEl) ? sayEl.GetString() : null;
            var text = el.TryGetProperty("text", out var txtEl) && txtEl.ValueKind == JsonValueKind.String ? txtEl.GetString() : null;

            if (ask is "command" or "tool" or "followup" or "plan_mode_respond")
            {
                var msg = string.IsNullOrEmpty(text) ? "等待你确认" : OneLine(text!)!;
                return new AgentSessionProbe(new AgentSessionSignal.Attention(
                    new AgentAttentionRequest(Fingerprint(path, ask + msg.Length), Trunc(msg, 80))));
            }
            if (say is "command_output" or "tool")
                return new AgentSessionProbe(new AgentSessionSignal.Active(
                    Fingerprint(path, say + (text?.Length ?? 0)), text == null ? null : $"运行: {Trunc(OneLine(text!)!, 72)}"));
            if (say == "completion_result")
                return new AgentSessionProbe(new AgentSessionSignal.Completed(Fingerprint(path, text?[..Math.Min(64, text.Length)] ?? say)));
            return new AgentSessionProbe(null);
        }
        catch (JsonException)
        {
            return new AgentSessionProbe(null,
                new SessionProbeHealth(SessionProbeFailure.UndecodableFile, path, DateTimeOffset.Now));
        }
    }

    // MARK: 工具函数

    private static List<string> ReadTailLines(string path)
    {
        using var fs = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.ReadWrite | FileShare.Delete);
        var length = fs.Length;
        var start = Math.Max(0, length - TailBytes);
        fs.Seek(start, SeekOrigin.Begin);
        using var reader = new StreamReader(fs, Encoding.UTF8, detectEncodingFromByteOrderMarks: false, bufferSize: 8192, leaveOpen: true);
        // 起始行大概率被截断：丢弃第一个不完整行
        if (start > 0) reader.ReadLine();
        var lines = new List<string>(256);
        while (reader.ReadLine() is { } line)
        {
            if (line.Length > 0) lines.Add(line);
            if (lines.Count > 600) lines.RemoveAt(0); // 有界
        }
        return lines;
    }

    private static string Fingerprint(string path, string key) => $"{path.GetHashCode(StringComparison.OrdinalIgnoreCase):x}:{key.GetHashCode(StringComparison.Ordinal):x}";

    private static string OneLine(string s) => Trunc(s.ReplaceLineEndings(" ⏎ ").Trim(), 100);

    private static string Trunc(string s, int max) => s.Length <= max ? s : s[..max] + "…";
}
