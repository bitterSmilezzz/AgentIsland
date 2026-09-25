using System.Diagnostics;
using System.IO;

namespace AgentIsland.Core;

/// 五态状态机引擎（与 macOS 端 ActivityEngine 同规则）：
/// · 未解决的确认/授权请求            → attention
/// · 本轮明确结束（有写入证据）        → completed
/// · 进程在 + 写入 60s 内 或 CPU≥阈值 → working
/// · 进程在但静默                      → idle
/// · 进程不在                          → offline（可见口径隐藏）
public sealed class ActivityEngine : IDisposable
{
    private readonly SettingsStore _settings;
    private readonly ProcessMonitor _processes = new();
    private readonly FileMonitor _files = new();
    private readonly SessionInspector _inspector = new();
    private readonly TokenUsageMonitor _tokens = new();

    private readonly Dictionary<string, AgentProfile> _profiles = [];
    private readonly Dictionary<string, DateTimeOffset> _workStartedAt = [];
    private readonly Dictionary<string, DateTimeOffset> _phaseSince = [];
    private readonly HashSet<string> _alertedFingerprints = [];
    private readonly Dictionary<string, string> _lastCompletedFp = [];
    private readonly Dictionary<string, (DateTimeOffset since, double lastCpu)> _highCpuSince = [];
    private readonly Dictionary<string, DateTimeOffset> _lastCostSpike = [];
    private readonly Dictionary<string, (string path, long length, DateTime mtime, AgentSessionProbe probe)> _probeCache = [];
    private readonly Dictionary<string, (DateTimeOffset fetched, TokenReport report)> _tokenCache = [];

    private CancellationTokenSource? _cts;
    private Task? _loop;
    private int _coreCount = Environment.ProcessorCount;

    public event Action? Changed;
    public event Action<AgentTaskEvent>? EventRaised;

    /// 演示模式（--demo）：注入假快照用于视觉验收与测试，不读真实进程
    public bool DemoMode { get; set; }

    private volatile AgentSnapshot[] _snapshots = [];
    private volatile AgentTaskEvent? _latestEvent;
    private TokenUsage _grandTotal = new();

    public AgentSnapshot[] Snapshots => _snapshots;
    public AgentSnapshot[] VisibleSnapshots => _snapshots.Where(s => s.ProcessRunning).ToArray();
    public AgentSnapshot[] RingShelfSnapshots => VisibleSnapshots
        .Where(s => s.Level is ActivityLevel.Attention or ActivityLevel.Working)
        .OrderByDescending(s => s.Level.Order())
        .Take(6)
        .ToArray();
    public AgentTaskEvent? LatestEvent => _latestEvent;
    public TokenUsage GrandTotal => _grandTotal;
    public bool AnyWorking => _snapshots.Any(s => s.Level == ActivityLevel.Working);
    public bool HasAttention => _snapshots.Any(s => s.Level == ActivityLevel.Attention);
    public DateTime LastTickAt { get; private set; } = DateTime.MinValue;

    public ActivityEngine(SettingsStore settings)
    {
        _settings = settings;
    }

    public void Start()
    {
        Stop();
        _cts = new CancellationTokenSource();
        _loop = Task.Run(() => RunAsync(_cts.Token));
    }

    public void Stop()
    {
        _cts?.Cancel();
        try { _loop?.Wait(1500); } catch { }
        _cts?.Dispose();
        _cts = null;
        _loop = null;
    }

    private async Task RunAsync(CancellationToken ct)
    {
        while (!ct.IsCancellationRequested)
        {
            var started = Stopwatch.GetTimestamp();
            try
            {
                await Task.Run(() => Tick(), ct);
            }
            catch (Exception)
            {
                // 单拍失败不终止引擎
            }
            var anyActive = _snapshots.Any(s => s.Level is ActivityLevel.Working or ActivityLevel.Attention);
            var interval = anyActive ? _settings.EngineConfig().SampleInterval : _settings.EngineConfig().IdleSampleInterval;
            var spent = Stopwatch.GetElapsedTime(started).TotalSeconds;
            var delay = Math.Max(0.2, interval - spent);
            try { await Task.Delay(TimeSpan.FromSeconds(delay), ct); }
            catch (OperationCanceledException) { break; }
        }
    }

    private void Tick()
    {
        var settings = _settings;
        var config = settings.EngineConfig();
        var profiles = AgentRegistry.EnabledProfiles(settings);
        var processTable = _processes.Take(out _);

        var list = new List<AgentSnapshot>(profiles.Count);
        long total24 = 0, totalAll = 0; double cost24 = 0, costAll = 0;
        var now = DateTimeOffset.Now;

        foreach (var profile in profiles)
        {
            _profiles[profile.Id] = profile;
            var (pid, memory) = ProcessMonitor.MatchProcess(processTable, profile, out var matchedPids);
            var processRunning = matchedPids.Count > 0;
            var cpu = _processes.CpuPercent(matchedPids, _coreCount);

            var fileResult = _files.Probe(profile, TimeSpan.FromSeconds(config.ActiveSessionWindow));
            var probe = ProbeCached(profile, fileResult.LatestFile);

            var level = DecideLevel(profile, config, now, processRunning, cpu, fileResult, probe, pid);

            var tokenUsage = TokenUsageCached(profile);
            if (tokenUsage != null)
            {
                total24 += tokenUsage.Tokens24h; totalAll += tokenUsage.TokensTotal;
                cost24 += tokenUsage.Cost24h; costAll += tokenUsage.CostTotal;
            }

            list.Add(new AgentSnapshot
            {
                Profile = profile,
                Level = level,
                ProcessRunning = processRunning,
                CpuPercent = cpu,
                Installed = true,
                LastActivityAgo = fileResult.LatestWrite is { } lw ? (DateTimeOffset.Now - lw).TotalSeconds : null,
                LastActivityText = fileResult.LatestWrite is { } lw2 ? TimeAgoText.Format(DateTimeOffset.Now - lw2) : "—",
                TokenUsage = tokenUsage,
                Pid = pid,
                CurrentAction = probe.Signal?.ActionText,
                MemoryBytes = memory,
                SubagentCount = probe.SubagentCount,
                BackgroundTaskCount = probe.BackgroundTaskCount,
                SessionProbeHealth = probe.Health,
            });
        }

        _snapshots = list
            .OrderByDescending(s => s.Level.Order())
            .ThenBy(s => s.Profile.Name, StringComparer.OrdinalIgnoreCase)
            .ToArray();
        _grandTotal = new TokenUsage
        {
            Tokens24h = total24,
            TokensTotal = totalAll,
            Cost24h = cost24,
            CostTotal = costAll,
        };
        if (DemoMode) ApplyDemoSnapshots();
        LastTickAt = DateTime.Now;
        Changed?.Invoke();
    }

    // MARK: 演示数据（对齐 macOS 端 site 截图的样例状态）

    private void ApplyDemoSnapshots()
    {
        static AgentProfile P(string id, string name, string glyph) => new()
        { Id = id, Name = name, Glyph = glyph, ProcessNames = ["demo"], };
        var profiles = new (AgentProfile p, ActivityLevel level, ulong mem, string? action, long t24)[]
        {
            (P("dim", "DimAgent", "\uE8FD"), ActivityLevel.Idle, 277ul * 1048576, null, 2_070_000),
            (P("qoder", "Qoder", "\uF471"), ActivityLevel.Working, 877ul * 1048576, "运行: chmod +x .scr.te-shots/shoot4.sh …", 1_200_000_000),
            (P("workbuddy", "WorkBuddy", "\uE721"), ActivityLevel.Idle, 322ul * 1048576, null, 2_760_000),
            (P("workbuddyai", "WorkBuddy AI", "\uE774"), ActivityLevel.Idle, 362ul * 1048576, null, 1_860_000),
            (P("antigravity", "Antigravity", "\uE943"), ActivityLevel.Working, 310ul * 1048576, "正在修改: IslandView.swift", 1_600_000_000),
            (P("chatgpt", "ChatGPT", "\uE99A"), ActivityLevel.Idle, 131ul * 1048576, null, 0),
        };
        var demo = profiles.Select(x => new AgentSnapshot
        {
            Profile = x.p,
            Level = x.level,
            ProcessRunning = true,
            CpuPercent = x.level == ActivityLevel.Working ? 34.0 : 1.2,
            Installed = true,
            LastActivityAgo = x.level == ActivityLevel.Working ? 3 : null,
            LastActivityText = x.level == ActivityLevel.Working ? "刚刚" : "—",
            TokenUsage = x.t24 > 0 ? new TokenUsage { Tokens24h = x.t24, TokensTotal = x.t24 * 23 } : null,
            Pid = 0,
            CurrentAction = x.action,
            MemoryBytes = x.mem,
        }).ToList();
        demo.Insert(0, new AgentSnapshot
        {
            Profile = P("claude", "Claude", "\uE8BD"),
            Level = ActivityLevel.Attention,
            ProcessRunning = true,
            CpuPercent = 8.0,
            Installed = true,
            LastActivityAgo = 5,
            LastActivityText = "刚刚",
            TokenUsage = new TokenUsage { Tokens24h = 12_080_000, TokensTotal = 12_080_000 },
            Pid = 0,
            CurrentAction = "需要确认: …ift test",
            MemoryBytes = 430ul * 1048576,
        });

        _snapshots = demo
            .OrderByDescending(s => s.Level.Order())
            .ThenBy(s => s.Profile.Name, StringComparer.OrdinalIgnoreCase)
            .ToArray();
        _grandTotal = new TokenUsage
        {
            Tokens24h = 12_080_000,
            TokensTotal = 280_000_000,
            Cost24h = 0.84,
            CostTotal = 19.37,
        };
        _latestEvent ??= new AgentTaskEvent
        {
            AgentId = "claude",
            AgentName = "dim",
            Type = AgentTaskEvent.EventType.Attention,
            Duration = 0,
            Message = "需要确认: 是否允许执行 swift test",
            Detail = "会话请求执行 shell 命令 swift test，等待用户批准。可在岛内直达终端或忽略。",
        };
    }

    // MARK: 五态判定

    private ActivityLevel DecideLevel(
        AgentProfile profile, EngineConfig config, DateTimeOffset now, bool processRunning,
        double? cpu, FileActivityResult file, AgentSessionProbe probe, int? pid)
    {
        var key = profile.Id;

        if (!processRunning)
        {
            _phaseSince[key] = now;
            return ActivityLevel.Offline;
        }

        // 强语义：attention 优先
        if (probe.Signal is AgentSessionSignal.Attention att)
        {
            var fp = att.Request.Fingerprint;
            if (_alertedFingerprints.Add(fp))
            {
                RaiseEvent(new AgentTaskEvent
                {
                    AgentId = key,
                    AgentName = profile.Name,
                    Type = AgentTaskEvent.EventType.Attention,
                    Duration = 0,
                    Pid = pid,
                    Message = att.Request.Message,
                });
                // 指纹集合有界
                if (_alertedFingerprints.Count > 800)
                    _alertedFingerprints.Clear();
            }
            _phaseSince[key] = now;
            return ActivityLevel.Attention;
        }

        // 强语义：本轮明确结束（有写入证据才宣布完成）
        if (probe.Signal is AgentSessionSignal.Completed done &&
            file.LatestWrite is { } lw &&
            now - lw < TimeSpan.FromSeconds(config.WorkingWindow))
        {
            var changed = _phaseSince.TryGetValue(key, out var since) ? (now - since).TotalSeconds : 0;
            if (_lastCompletedFp.TryGetValue(key, out var lastFp) ? lastFp != done.Fingerprint : true)
            {
                _lastCompletedFp[key] = done.Fingerprint;
                RaiseEvent(new AgentTaskEvent
                {
                    AgentId = key,
                    AgentName = profile.Name,
                    Type = AgentTaskEvent.EventType.Completed,
                    Duration = Math.Max(0, changed),
                    Pid = pid,
                });
            }
            _phaseSince[key] = now;
            return ActivityLevel.Completed;
        }

        var writeEvidence = file.LatestWrite is { } w && now - w < TimeSpan.FromSeconds(config.WorkingWindow);
        var cpuEvidence = (cpu ?? 0) >= profile.CpuWorkingThreshold
            ? Math.Max(profile.CpuWorkingThreshold ?? 0, config.CpuThreshold)
            : config.CpuThreshold;
        var cpuHot = cpu.HasValue && cpu.Value >= cpuEvidence;

        // 在途命令全周期拦截：pending tool_use 期间严格 working
        var inFlight = probe.Signal is AgentSessionSignal.Active;

        ActivityLevel level;
        if (inFlight || writeEvidence || cpuHot)
        {
            // 滞回：working 信号消失后保持最短时长（防抖动）
            var holding = _phaseSince.TryGetValue(key, out var s0) &&
                          _snapshots.Any(x => x.Id == key && x.Level == ActivityLevel.Working) &&
                          now - s0 < TimeSpan.FromSeconds(config.MinWorkingHold);
            level = ActivityLevel.Working;
            if (!_workStartedAt.ContainsKey(key)) _workStartedAt[key] = now;
            _phaseSince[key] = now;
        }
        else
        {
            level = ActivityLevel.Idle;
            _workStartedAt.Remove(key);
            _phaseSince[key] = now;
        }

        // 死循环熔断：CPU 连续 70% 以上达 5 分钟
        if (cpu.HasValue)
        {
            if (cpu.Value >= config.RunawayCpuThreshold)
            {
                var since = _highCpuSince.TryGetValue(key, out var h) ? h.since : now;
                _highCpuSince[key] = (since, cpu.Value);
                if (config.RunawayCpuAlert &&
                    now - since > TimeSpan.FromSeconds(config.RunawayDurationThreshold))
                    RaiseCostSpike(profile, pid, $"CPU 持续 {cpu.Value:0}% 已超过 {(int)(now - since).TotalMinutes} 分钟", now);
            }
            else
            {
                _highCpuSince.Remove(key);
            }
        }

        // Token 暴涨：1 分钟增量超阈值（粗粒度：24h 增量斜率代理，10 分钟一次上限）
        if (config.TokenAlertEnabled)
        {
            var usage = TokenUsageCached(profile);
            if (usage is { } u && u.Tokens24h > config.TokenAlertThreshold)
            {
                // 每拍全量累计无法直接折算分钟增量；以「24h 内首次越过阈值」为一次告警
                RaiseCostSpike(profile, pid, $"24h Token 用量已达 {TokenUsage.Compact(u.Tokens24h)}", now, once: "token-threshold");
            }
        }

        return level;
    }

    private void RaiseCostSpike(AgentProfile profile, int? pid, string message, DateTimeOffset now, string? once = "cpu")
    {
        var dedupeKey = $"{profile.Id}:{once}";
        if (_lastCostSpike.TryGetValue(dedupeKey, out var last) &&
            now - last < TimeSpan.FromSeconds(600))
            return;
        _lastCostSpike[dedupeKey] = now;
        if (_lastCostSpike.Count > 200) _lastCostSpike.Clear();
        RaiseEvent(new AgentTaskEvent
        {
            AgentId = profile.Id,
            AgentName = profile.Name,
            Type = AgentTaskEvent.EventType.CostSpike,
            Duration = 0,
            Pid = pid,
            Message = message,
        });
    }

    private void RaiseEvent(AgentTaskEvent e)
    {
        _latestEvent = e;
        EventRaised?.Invoke(e);
        Changed?.Invoke();
    }

    /// 外部投递事件（/notify 深链等），岛内必须看得出来
    public void RaiseExternalEvent(string agentIdOrName, AgentTaskEvent.EventType type, string? message, string? detail)
    {
        var profile = _profiles.Values.FirstOrDefault(p =>
            p.Id.Equals(agentIdOrName, StringComparison.OrdinalIgnoreCase) ||
            p.Name.Equals(agentIdOrName, StringComparison.OrdinalIgnoreCase));
        var agentId = profile?.Id ?? agentIdOrName;
        var agentName = profile?.Name ?? agentIdOrName;
        RaiseEvent(new AgentTaskEvent
        {
            AgentId = agentId,
            AgentName = agentName,
            Type = type,
            Duration = 0,
            Message = message,
            Detail = detail,
            ExternallyDelivered = true,
        });
    }

    /// 用户关闭横幅：只清「最新事件」指针，不改会话状态
    public void ClearLatestEvent()
    {
        _latestEvent = null;
        Changed?.Invoke();
    }

    /// 一键终止逃生舱：按 pid + 进程名复核身份后终止（pid 已被系统回收复用时拒绝执行）
    public void TerminateAgent(int? pid, string agentId)
    {
        if (pid is not { } p) return;
        var profile = _profiles.GetValueOrDefault(agentId);
        try
        {
            var proc = System.Diagnostics.Process.GetProcessById(p);
            // 身份复核：进程名仍要匹配该档案（pid 复用防护）
            if (profile != null && profile.ProcessNames.Length > 0 &&
                !profile.ProcessNames.Any(n => proc.ProcessName.StartsWith(n, StringComparison.OrdinalIgnoreCase)))
                return;
            proc.Kill(entireProcessTree: true);
        }
        catch
        {
            // 进程已退出：目标本就不在了
        }
    }

    // MARK: 探测缓存

    private AgentSessionProbe ProbeCached(AgentProfile profile, string? path)
    {
        if (path == null) return new AgentSessionProbe(null);
        long length = 0;
        DateTime mtime = DateTime.MinValue;
        try
        {
            var fi = new FileInfo(path);
            if (!fi.Exists) return new AgentSessionProbe(null);
            length = fi.Length;
            mtime = fi.LastWriteTimeUtc;
        }
        catch (IOException) { return new AgentSessionProbe(null); }

        if (_probeCache.TryGetValue(path, out var cached) &&
            cached.length == length && cached.mtime == mtime)
            return cached.probe;

        var fileResult = new FileActivityResult(null, path, 0);
        var probe = _inspector.Probe(profile, fileResult);
        if (_probeCache.Count > 200) _probeCache.Clear();
        _probeCache[path] = (path, length, mtime, probe);
        return probe;
    }

    private TokenUsage? TokenUsageCached(AgentProfile profile)
    {
        if (profile.TokenRoots.Length == 0) return null;
        if (_tokenCache.TryGetValue(profile.Id, out var c) &&
            DateTimeOffset.Now - c.fetched < TimeSpan.FromSeconds(20))
            return c.report.Usage;
        try
        {
            var report = _tokens.Monitor(profile);
            _tokenCache[profile.Id] = (DateTimeOffset.Now, report);
            return report.Usage;
        }
        catch (Exception)
        {
            return null;
        }
    }

    /// 供详情页/分析页读取按模型拆分（带缓存）
    public TokenReport? GetTokenReport(string agentId)
    {
        if (DemoMode) return DemoReport();
        if (_tokenCache.TryGetValue(agentId, out var c)) return c.report;
        if (!_profiles.TryGetValue(agentId, out var profile)) return null;
        try
        {
            var report = _tokens.Monitor(profile);
            _tokenCache[agentId] = (DateTimeOffset.Now, report);
            return report;
        }
        catch (Exception)
        {
            return null;
        }
    }

    private TokenReport? _demoReport;

    /// 演示模式的报表（确定性伪随机，峰值 2.83M）
    private TokenReport DemoReport()
    {
        if (_demoReport != null) return _demoReport;
        var hourly = new List<HourBucket>();
        var rng = new Random(42);
        var now = DateTime.Now;
        for (var i = 24 * 30; i >= 0; i--)
        {
            var h = new DateTime(now.Year, now.Month, now.Day, now.Hour, 0, 0).AddHours(-i);
            var v = (long)(400_000 * Math.Max(0.05, Math.Sin(i / 5.0) * 0.5 + 0.5));
            if (rng.NextDouble() < 0.35) v = 0;
            hourly.Add(new HourBucket(h, v));
        }
        var peak = hourly[^3];
        hourly[^3] = new HourBucket(peak.Hour, 2_830_000);
        var models = new List<ModelUsage>
        {
            new("codex-5", 5_340_000, 6.10),
            new("claude-sonnet-4-5", 2_760_000, 3.22),
            new("claude-opus-4", 2_070_000, 9.41),
            new("gpt-4o", 1_860_000, 2.05),
            new("claude-haiku-4", 489_000, 0.31),
        };
        _demoReport = new TokenReport(
            new TokenUsage { Tokens24h = 12_080_000, TokensTotal = 280_000_000, Cost24h = 0.84, CostTotal = 19.37 },
            models, models, hourly);
        return _demoReport;
    }

    public void Dispose()
    {
        Stop();
    }
}
