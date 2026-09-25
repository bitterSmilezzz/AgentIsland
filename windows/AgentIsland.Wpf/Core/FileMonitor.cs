using System.IO;

namespace AgentIsland.Core;

public sealed record FileActivityResult(DateTimeOffset? LatestWrite, string? LatestFile, int ActiveSessions);

/// 会话目录后台扫描：找最近写入的会话文件 + 判定窗口内写入证据。
/// 目录结果带节流缓存（引擎 2s 一拍，目录树全量重扫太贵）。
public sealed class FileMonitor
{
    private const int MaxDepth = 3;
    private const int MaxFilesPerScan = 4000;

    private sealed class DirCache
    {
        public DateTimeOffset LastScan;
        public DateTimeOffset? LatestWrite;
        public string? LatestFile;
    }

    private readonly Dictionary<string, DirCache> _cache = [];
    private readonly object _lock = new();

    /// 单目录（含子目录）最新写入。缓存 TTL：刚活动过 2s，安静 6s。
    public FileActivityResult Probe(AgentProfile profile, TimeSpan activeSessionWindow)
    {
        DateTimeOffset? latest = null;
        string? latestFile = null;
        var activeSessions = 0;
        var cutoff = DateTimeOffset.Now - activeSessionWindow;

        foreach (var root in profile.SessionDirs)
        {
            if (!Directory.Exists(root)) continue;
            var r = ProbeDir(root, DateTime.UtcNow);
            if (r.LatestWrite > latest)
            {
                latest = r.LatestWrite;
                latestFile = r.LatestFile;
            }
            try
            {
                foreach (var f in Directory.EnumerateFiles(root, "*", SearchOption.AllDirectories))
                {
                    var mtime = File.GetLastWriteTimeUtc(f);
                    if (mtime >= cutoff.UtcDateTime) activeSessions++;
                    if (activeSessions > 99) break;
                }
            }
            catch (IOException) { }
            catch (UnauthorizedAccessException) { }
        }

        double? ago = latest is { } lw ? (DateTimeOffset.Now - lw).TotalSeconds : null;
        return new FileActivityResult(latest, latestFile, activeSessions) with { };
    }

    private DirCache ProbeDir(string dir, DateTime utcNow)
    {
        lock (_lock)
        {
            if (_cache.TryGetValue(dir, out var cached))
            {
                var ttl = (DateTimeOffset.Now - cached.LatestWrite) < TimeSpan.FromSeconds(30)
                    ? TimeSpan.FromSeconds(2) : TimeSpan.FromSeconds(6);
                if (DateTimeOffset.Now - cached.LastScan < ttl) return cached;
            }
            else
            {
                cached = new DirCache();
                _cache[dir] = cached;
            }

            DateTimeOffset? latest = null;
            string? latestFile = null;
            try
            {
                var count = 0;
                foreach (var f in EnumerateFilesSafe(dir))
                {
                    if (++count > MaxFilesPerScan) break;
                    var ext = Path.GetExtension(f);
                    if (ext is not (".jsonl" or ".json" or ".db" or ".sqlite" or ".log")) continue;
                    DateTimeOffset mtime;
                    try { mtime = File.GetLastWriteTimeUtc(f); } catch (IOException) { continue; }
                    if (latest == null || mtime > latest)
                    {
                        latest = mtime;
                        latestFile = f;
                    }
                }
            }
            catch (IOException) { }
            catch (UnauthorizedAccessException) { }

            cached.LastScan = DateTimeOffset.Now;
            cached.LatestWrite = latest;
            cached.LatestFile = latestFile;
            return cached;
        }
    }

    private static IEnumerable<string> EnumerateFilesSafe(string root)
    {
        var pending = new List<(string dir, int depth)> { (root, 0) };
        while (pending.Count > 0)
        {
            var (dir, depth) = pending[^1];
            pending.RemoveAt(pending.Count - 1);

            string[] files = [];
            string[] subDirs = [];
            try
            {
                files = Directory.GetFiles(dir);
                subDirs = Directory.GetDirectories(dir);
            }
            catch (IOException) { }
            catch (UnauthorizedAccessException) { }

            foreach (var f in files) yield return f;

            if (depth >= MaxDepth) continue;
            foreach (var sub in subDirs)
            {
                var name = Path.GetFileName(sub);
                // 跳过依赖树与噪声目录（与 macOS 端同规则）
                if (name is "node_modules" or ".git" or "Cache" or "CachedData" or "GPUCache" or "logs") continue;
                pending.Add((sub, depth + 1));
            }
        }
    }

    /// 引擎重置（会话目录配置变更时）
    public void Reset() { lock (_lock) _cache.Clear(); }
}
