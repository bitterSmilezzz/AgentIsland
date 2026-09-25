using System.IO;

namespace AgentIsland.Core;

/// Agent 注册表：内置集（Windows 路径）+ 启用状态（来自设置）。
/// 与 macOS 端 AgentRegistry 同构；路径一律用环境变量展开的绝对路径。
public static class AgentRegistry
{
    private static string Home => Environment.GetFolderPath(Environment.SpecialFolder.UserProfile);
    private static string AppData => Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData);
    private static string LocalAppData => Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData);

    /// 桌面类（Electron/多进程）Agent 的 CPU 判定下限：空闲渲染抖动 4%~15%
    private const double DesktopCpuFloor = 20.0;

    public static IReadOnlyList<AgentProfile> Builtin { get; } =
    [
        new()
        {
            Id = "claude",
            Name = "Claude",
            Glyph = "\uE8BD", // Message
            ProcessNames = ["claude"],
            CpuWorkingThreshold = null, // CLI 档：走全局阈值
            SessionDirs = [Path.Combine(Home, ".claude", "projects"), Path.Combine(Home, ".claude", "sessions")],
            TokenRoots = [Path.Combine(Home, ".claude", "projects")],
            Category = "assistant",
            Emoji = "🧠",
        },
        new()
        {
            Id = "codex",
            Name = "Codex",
            Glyph = "\uE968", // Robot
            ProcessNames = ["codex"],
            SessionDirs = [Path.Combine(Home, ".codex", "sessions")],
            TokenRoots = [Path.Combine(Home, ".codex", "sessions")],
            Category = "assistant",
            Emoji = "🤖",
        },
        new()
        {
            Id = "cursor",
            Name = "Cursor",
            Glyph = "\uE762", // TouchPointer
            ProcessNames = ["Cursor"],
            CpuWorkingThreshold = DesktopCpuFloor,
            SessionDirs = [Path.Combine(AppData, "Cursor", "User", "workspaceStorage")],
            Category = "codeEditor",
            Emoji = "💻",
        },
        new()
        {
            Id = "vscode",
            Name = "VS Code",
            Glyph = "\uE943", // Code
            ProcessNames = ["Code"],
            CpuWorkingThreshold = DesktopCpuFloor,
            SessionDirs = [Path.Combine(AppData, "Code", "User", "workspaceStorage")],
            Category = "codeEditor",
            Emoji = "⌨️",
        },
        new()
        {
            Id = "cline",
            Name = "Cline",
            Glyph = "\uE756", // CommandPrompt
            ProcessNames = [],
            SessionDirs = [Path.Combine(AppData, "Code", "User", "globalStorage", "saoudrizwan.claude-dev", "tasks")],
            TokenRoots = [Path.Combine(AppData, "Code", "User", "globalStorage", "saoudrizwan.claude-dev", "tasks")],
            Category = "assistant",
            Emoji = "🔗",
        },
        new()
        {
            Id = "roo",
            Name = "Roo Code",
            Glyph = "\uE8A5", // Document
            ProcessNames = [],
            SessionDirs = [Path.Combine(AppData, "Code", "User", "globalStorage", "rooveterinaryinc.roo-cline", "tasks")],
            TokenRoots = [Path.Combine(AppData, "Code", "User", "globalStorage", "rooveterinaryinc.roo-cline", "tasks")],
            Category = "assistant",
            Emoji = "🦘",
        },
        new()
        {
            Id = "opencode",
            Name = "OpenCode",
            Glyph = "\uE8E5", // NavigateExternal
            ProcessNames = ["opencode"],
            SessionDirs = [Path.Combine(Home, ".local", "share", "opencode")],
            TokenRoots = [Path.Combine(Home, ".local", "share", "opencode")],
            Category = "assistant",
            Emoji = "📂",
        },
        new()
        {
            Id = "goose",
            Name = "Goose",
            Glyph = "\uE7BB", // World
            ProcessNames = ["goose"],
            SessionDirs = [Path.Combine(Home, ".config", "goose", "sessions")],
            Category = "assistant",
            Emoji = "🪿",
        },
        new()
        {
            Id = "aider",
            Name = "Aider",
            Glyph = "\uE943", // Code
            ProcessNames = ["aider"],
            SessionDirs = [Path.Combine(Home, ".aider")],
            Category = "assistant",
            Emoji = "🛠️",
        },
        new()
        {
            Id = "continue",
            Name = "Continue",
            Glyph = "\uE72C", // Refresh
            ProcessNames = ["continue"],
            SessionDirs = [Path.Combine(Home, ".continue")],
            Category = "assistant",
            Emoji = "↩️",
        },
        new()
        {
            Id = "windsurf",
            Name = "Windsurf",
            Glyph = "\uE7C3", // Page
            ProcessNames = ["Windsurf"],
            CpuWorkingThreshold = DesktopCpuFloor,
            SessionDirs = [Path.Combine(Home, ".codeium", "windsurf")],
            Category = "codeEditor",
            Emoji = "🏄",
        },
        new()
        {
            Id = "trae",
            Name = "Trae",
            Glyph = "\uE7C4", // PageList? 近似
            ProcessNames = ["Trae"],
            CpuWorkingThreshold = DesktopCpuFloor,
            SessionDirs = [Path.Combine(AppData, "Trae", "User", "workspaceStorage"), Path.Combine(AppData, "Trae CN", "User", "workspaceStorage")],
            Category = "codeEditor",
            Emoji = "🛳️",
        },
    ];

    /// 引擎实际监控的档案：内置集中已安装且未被用户禁用的那些。
    /// 「安装」= 任一 processName 命中或任一会话目录存在——离线项反正会被可见口径隐藏，
    /// 但没安装的档案不该占 doctor 与分析页的篇幅。
    public static IReadOnlyList<AgentProfile> EnabledProfiles(SettingsStore settings)
    {
        var list = new List<AgentProfile>();
        foreach (var p in Builtin)
        {
            if (settings.DisabledAgents.Contains(p.Id)) continue;
            if (!IsInstalled(p)) continue;
            list.Add(p);
        }
        return list;
    }

    public static bool IsInstalled(AgentProfile p)
    {
        foreach (var dir in p.SessionDirs)
            if (Directory.Exists(dir)) return true;
        return p.ProcessNames.Length > 0; // 有进程名可查的档案交给引擎运行时判定
    }
}
