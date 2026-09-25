using System.Diagnostics;

namespace AgentIsland.Core;

public sealed record ProcessInfo(int Pid, string Name, string ExePath, ulong MemoryBytes);

/// 进程表快照 + CPU 差分（两次采样之间的进程时间差分量）。
/// CPU% 是窗口量：第一拍结构性测不出，返回 null 而不是 0。
public sealed class ProcessMonitor
{
    private readonly Dictionary<int, (TimeSpan totalCpu, DateTime at)> _lastSample = [];
    private Dictionary<int, ProcessInfo> _lastProcesses = [];

    /// 拿一次快照。返回全部进程；CPU 利用率由调用方按档案进程名聚合后读取。
    public Dictionary<int, ProcessInfo> Take(out double? deltaSeconds)
    {
        deltaSeconds = null;
        var result = new Dictionary<int, ProcessInfo>(256);
        var now = DateTime.UtcNow;
        var sample = new Dictionary<int, (TimeSpan, DateTime)>(256);

        foreach (var proc in Process.GetProcesses())
        {
            try
            {
                string? path = null;
                try { path = proc.MainModule?.FileName; } catch { /* 系统进程拒绝访问 */ }
                var name = proc.ProcessName;
                result[proc.Id] = new ProcessInfo(proc.Id, name, path ?? "",
                    (ulong)Math.Max(0, proc.WorkingSet64));
                sample[proc.Id] = (proc.TotalProcessorTime, now);
            }
            catch (Exception) when (proc is { HasExited: true } or null)
            {
                // 进程恰好在枚举间隙退出
            }
            catch
            {
                // 访问被拒（其他用户的进程）：仍记录名字与 PID，CPU 时间拿不到
                try
                {
                    result[proc.Id] = new ProcessInfo(proc.Id, proc.ProcessName, "", 0);
                }
                catch { /* 彻底拿不到就算了 */ }
            }
            finally
            {
                proc.Dispose();
            }
        }

        var elapsed = _lastSample.Count > 0 ? (now - _lastSample.Values.Min(s => s.at)).TotalSeconds : 0;
        if (elapsed > 0.2)
        {
            deltaSeconds = elapsed;
            // 保存各进程上一拍 CPU 总量供 CPU% 计算
            _cpuPrev = _lastSample.ToDictionary(kv => kv.Key, kv => kv.Value.totalCpu);
            _lastDeltaSeconds = elapsed;
        }

        _lastSample.Clear();
        foreach (var kv in sample) _lastSample[kv.Key] = kv.Value;
        _lastProcesses = result;
        return result;
    }

    private Dictionary<int, TimeSpan> _cpuPrev = [];
    private double _lastDeltaSeconds;

    /// 对最近两次快照计算某进程集合的合计 CPU 利用率（0~100）。无窗口时返回 null。
    public double? CpuPercent(IEnumerable<int> pids, int coreCount)
    {
        if (_cpuPrev.Count == 0 || _lastDeltaSeconds <= 0 || coreCount <= 0) return null;
        double total = 0;
        var any = false;
        foreach (var pid in pids)
        {
            if (!_cpuPrev.TryGetValue(pid, out var prev)) continue;
            var cur = _lastSample.TryGetValue(pid, out var s) ? s.totalCpu : TimeSpan.Zero;
            total += (cur - prev).TotalSeconds;
            any = true;
        }
        if (!any) return null;
        var pct = total / (_lastDeltaSeconds * coreCount) * 100.0;
        return Math.Clamp(pct, 0, 100 * coreCount);
    }

    /// 供引擎判定进程是否在跑：名字前缀匹配 + 路径包含/排除修正
    public static (int? pid, ulong memory) MatchProcess(
        Dictionary<int, ProcessInfo> processes,
        AgentProfile profile,
        out List<int> matchedPids)
    {
        matchedPids = [];
        int? bestPid = null;
        ulong bestMem = 0;
        foreach (var p in processes.Values)
        {
            if (!NameMatches(p.Name, profile)) continue;
            var exe = p.ExePath ?? "";
            if (profile.PathExcludes.Any(x => exe.Contains(x, StringComparison.OrdinalIgnoreCase))) continue;
            if (profile.PathContains.Length > 0 &&
                !profile.PathContains.Any(x => exe.Contains(x, StringComparison.OrdinalIgnoreCase)))
                continue;
            matchedPids.Add(p.Pid);
            if (p.MemoryBytes > bestMem)
            {
                bestMem = p.MemoryBytes;
                bestPid = p.Pid;
            }
        }
        return (bestPid, bestMem);
    }

    private static bool NameMatches(string processName, AgentProfile profile)
    {
        foreach (var want in profile.ProcessNames)
        {
            // 「name + 空格」前缀族规则：命中 "qoder helper (renderer)" 一类
            if (processName.Equals(want, StringComparison.OrdinalIgnoreCase) ||
                processName.StartsWith(want + " ", StringComparison.OrdinalIgnoreCase) ||
                processName.StartsWith(want + "(", StringComparison.OrdinalIgnoreCase))
                return true;
        }
        return false;
    }
}
