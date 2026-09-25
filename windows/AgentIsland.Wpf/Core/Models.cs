namespace AgentIsland.Core;

// MARK: - 活动等级（五态状态机）
// 与 macOS 端 AgentIslandCore/Models.swift 的 ActivityLevel 一一对应。

public enum ActivityLevel
{
    Offline,
    Idle,
    Completed,
    Working,
    Attention,
}

public static class ActivityLevelExtensions
{
    public static string Label(this ActivityLevel level) => level switch
    {
        ActivityLevel.Offline => "离线",
        ActivityLevel.Idle => "待机",
        ActivityLevel.Completed => "已完成",
        ActivityLevel.Working => "工作中",
        ActivityLevel.Attention => "待确认",
        _ => "未知",
    };

    public static int Order(this ActivityLevel level) => level switch
    {
        ActivityLevel.Offline => 0,
        ActivityLevel.Idle => 1,
        ActivityLevel.Completed => 2,
        ActivityLevel.Working => 3,
        ActivityLevel.Attention => 4,
        _ => 0,
    };
}

// MARK: - 贴边停靠模式

public enum DockEdge
{
    Right,
    Top,
    Bottom,
    Left,
}

public static class DockEdgeExtensions
{
    public static string Label(this DockEdge edge) => edge switch
    {
        DockEdge.Right => "右侧边栏",
        DockEdge.Top => "顶部灵动岛",
        DockEdge.Bottom => "底部停靠条",
        DockEdge.Left => "左侧边栏",
        _ => "右侧边栏",
    };

    /// 水平边缘（顶部/底部）沿 X 轴保存位置；垂直边缘沿 Y 轴保存位置。
    public static bool IsHorizontal(this DockEdge edge) => edge is DockEdge.Top or DockEdge.Bottom;
}

// MARK: - 外观模式

public enum IslandAppearance
{
    System,
    Light,
    Dark,
}

// MARK: - 通知分级

public enum NotificationPolicy
{
    Standard,
    Focus,
    Silent,
}

// MARK: - Agent 定义（注册表条目）
// 与 macOS 端 AgentProfile 同构；会话/Token 目录换成 Windows 路径。

public sealed class AgentProfile
{
    public required string Id { get; init; }
    public required string Name { get; init; }
    /// Segoe MDL2/Fluent 图标字形（等价于 macOS 的 SF Symbol 名）
    public required string Glyph { get; init; }
    public string[] ProcessNames { get; init; } = [];
    /// 可执行路径子串（区分同名宿主内嵌二进制）
    public string[] PathContains { get; init; } = [];
    /// 可执行路径排除子串：命中即不算本 Agent
    public string[] PathExcludes { get; init; } = [];
    /// 本 Agent 的 CPU 工作判定下限（null 表示无下限）
    public double? CpuWorkingThreshold { get; init; }
    public string[] SessionDirs { get; init; } = [];
    /// Token 明细根目录：目录树下的 JSONL 承载本 Agent 的 token 用量
    public string[] TokenRoots { get; init; } = [];
    public string Emoji { get; init; } = "🤖";
    public string Category { get; init; } = "assistant";
    public bool DefaultEnabled { get; init; } = true;
    public bool IsCustom { get; init; }

    /// 登记了任何本地明细源（token 目录）
    public bool HasLocalDetailSource => TokenRoots.Length > 0;
}

// MARK: - Token 用量

public sealed record TokenUsage
{
    public long Tokens24h { get; init; }
    public long TokensTotal { get; init; }
    public double Cost24h { get; init; }
    public double CostTotal { get; init; }

    public static string Compact(long tokens)
    {
        double n = tokens;
        if (tokens < 0) return "0";
        if (n < 1_000) return tokens.ToString("0");
        if (n < 1_000_000) return FormattableString.Invariant($"{n / 1_000d:0.#}k");
        if (n < 1_000_000_000)
        {
            var m = n / 1_000_000d;
            return m >= 100 ? FormattableString.Invariant($"{m:0}M") : FormattableString.Invariant($"{m:0.00}M");
        }
        return FormattableString.Invariant($"{n / 1_000_000_000d:0.##}G");
    }

    /// 花费文案；零花费返回空串（调用点据此隐藏）
    public static string Cost(double cost) => cost > 0.005 ? $"${cost:0.00}" : "";
}

// MARK: - 会话语义信号（强语义优先于 CPU/mtime 近似值）

public sealed record AgentAttentionRequest(string Fingerprint, string Message);

public abstract record AgentSessionSignal
{
    public sealed record Attention(AgentAttentionRequest Request) : AgentSessionSignal;
    public sealed record Completed(string Fingerprint) : AgentSessionSignal;
    public sealed record Active(string Fingerprint, string? Action) : AgentSessionSignal;

    public string? Fingerprint => this switch
    {
        Attention a => a.Request.Fingerprint,
        Completed c => c.Fingerprint,
        Active act => act.Fingerprint,
        _ => null,
    };

    public string? ActionText => this is Active a ? a.Action : null;
}

/// 会话探测失败原因（「没读到源」与「真的空闲」是两件事，不得混报）
public enum SessionProbeFailure
{
    UnreadableFile,
    UndecodableFile,
}

public sealed record SessionProbeHealth(SessionProbeFailure Failure, string Path, DateTimeOffset ObservedAt)
{
    public string DiagnosticText => $"会话源不可读：{Failure switch
    {
        SessionProbeFailure.UnreadableFile => "会话文件无法读取",
        SessionProbeFailure.UndecodableFile => "会话文件格式与解析器不匹配",
        _ => "未知原因",
    }}（{Path}）";
}

public sealed record AgentSessionProbe(
    AgentSessionSignal? Signal,
    SessionProbeHealth? Health = null,
    int SubagentCount = 0,
    int BackgroundTaskCount = 0);

// MARK: - 实时快照（引擎输出）

public sealed class AgentSnapshot
{
    public required AgentProfile Profile { get; init; }
    public required ActivityLevel Level { get; init; }
    public bool ProcessRunning { get; init; }
    /// null 表示本轮没有差分窗口测不出（不谎报 0.0%）
    public double? CpuPercent { get; init; }
    public bool Installed { get; init; }
    public double? LastActivityAgo { get; init; }
    public string LastActivityText { get; init; } = "—";
    public TokenUsage? TokenUsage { get; init; }
    public int? Pid { get; init; }
    public string? CurrentAction { get; init; }
    public ulong MemoryBytes { get; init; }
    public bool? IsHung { get; init; }
    public int SubagentCount { get; init; }
    public int BackgroundTaskCount { get; init; }
    public SessionProbeHealth? SessionProbeHealth { get; init; }

    public string Id => Profile.Id;

    public string MemoryText => MemoryBytes switch
    {
        0 => "—",
        var b when b < 1_048_576 => "<1M",
        var b when b >= 1_073_741_824 => FormattableString.Invariant($"{b / 1_073_741_824d:0.#}G"),
        var b => $"{(long)(b / 1_048_576d)}M",
    };
}

// MARK: - 任务事件（生命周期关键节点）

public sealed class AgentTaskEvent
{
    public enum EventType { Completed, Attention, CostSpike }

    public Guid Id { get; } = Guid.NewGuid();
    public required string AgentId { get; init; }
    public required string AgentName { get; init; }
    public required EventType Type { get; init; }
    public double Duration { get; init; }
    public DateTimeOffset Timestamp { get; init; } = DateTimeOffset.Now;
    public int? Pid { get; init; }
    public string? Message { get; init; }
    public string? Detail { get; init; }
    /// 由外部投递（/notify、/session 无令牌降级）而非引擎自己判定，岛内必须看得出来
    public bool ExternallyDelivered { get; init; }

    public static string DurationText(double duration)
    {
        var seconds = Math.Max(0, (int)Math.Round(duration));
        return seconds >= 60 ? $"{seconds / 60}分{seconds % 60}秒" : $"{seconds}秒";
    }

    public string SummaryText => !string.IsNullOrEmpty(Message) ? Message : Type switch
    {
        EventType.Completed => $"{AgentName} 任务完成 ({DurationText(Duration)})",
        EventType.Attention => $"{AgentName} 等待确认操作",
        EventType.CostSpike => $"⚠️ {AgentName} 资源/Token 消耗突增",
        _ => "",
    };
}

// MARK: - 引擎参数（与 macOS EngineConfig 同源同钳制）

public sealed record EngineConfig
{
    public double SampleInterval { get; set; } = 2.0;
    public double IdleSampleInterval { get; set; } = 5.0;
    public double WorkingWindow { get; set; } = 60.0;
    public double CpuThreshold { get; set; } = 6.0;
    public double ActiveSessionWindow { get; set; } = 600.0;
    public double MinWorkingHold { get; set; } = 10.0;
    public bool TokenAlertEnabled { get; set; } = true;
    public int TokenAlertThreshold { get; set; } = 200_000;
    public bool RunawayCpuAlert { get; set; } = true;
    public double RunawayCpuThreshold { get; set; } = 70.0;
    public double RunawayDurationThreshold { get; set; } = 300.0;

    public EngineConfig Normalized()
    {
        var c = this;
        double Clamp(double v, double lo, double hi) => Math.Min(Math.Max(double.IsNaN(v) ? lo : v, lo), hi);
        c.CpuThreshold = Clamp(c.CpuThreshold, 1, 50);
        c.SampleInterval = Clamp(c.SampleInterval, 0.5, 600);
        c.IdleSampleInterval = Clamp(c.IdleSampleInterval, 0.5, 600);
        c.WorkingWindow = Clamp(c.WorkingWindow, 10, 300);
        c.ActiveSessionWindow = Clamp(c.ActiveSessionWindow, 60, 3600);
        c.MinWorkingHold = Clamp(c.MinWorkingHold, 1, 300);
        c.RunawayCpuThreshold = Clamp(c.RunawayCpuThreshold, 10, 100);
        c.RunawayDurationThreshold = Clamp(c.RunawayDurationThreshold, 30, 3600);
        c.TokenAlertThreshold = (int)Math.Clamp(c.TokenAlertThreshold, 1_000, 10_000_000);
        if (c.SampleInterval > c.IdleSampleInterval) c.SampleInterval = c.IdleSampleInterval;
        return c;
    }
}

// MARK: - 文案工具

public static class TimeAgoText
{
    public static string Format(TimeSpan ago)
    {
        if (ago.TotalSeconds < 10) return "刚刚";
        if (ago.TotalMinutes < 1) return $"{(int)ago.TotalSeconds}秒前";
        if (ago.TotalMinutes < 60) return $"{(int)ago.TotalMinutes}分钟前";
        if (ago.TotalHours < 24) return $"{(int)ago.TotalHours}小时前";
        return $"{(int)ago.TotalDays}天前";
    }
}
