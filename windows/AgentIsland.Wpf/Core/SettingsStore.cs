using System.IO;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace AgentIsland.Core;

/// 持久化设置（%APPDATA%\AgentIsland\settings.json）。
/// macOS 端用 UserDefaults；Windows 端以 JSON 落盘，读写全经此类。
public sealed class SettingsStore
{
    public static SettingsStore Instance { get; } = new();

    public string Appearance { get; set; } = nameof(IslandAppearance.System);
    public string DockEdge { get; set; } = nameof(Core.DockEdge.Top);
    /// 贴边锚点：水平边为 X、垂直边为 Y（DIP，相对工作区起点）
    public double DockAnchor { get; set; } = 0.5;   // 0..1 比例
    public double CollapseDelay { get; set; } = 0.5;
    public double SampleInterval { get; set; } = 2.0;
    public double CpuThreshold { get; set; } = 6.0;
    public bool TokenAlertEnabled { get; set; } = true;
    public int TokenAlertThreshold { get; set; } = 200_000;
    public string NotificationPolicy { get; set; } = nameof(Core.NotificationPolicy.Standard);
    public bool PlayCompletionSound { get; set; } = true;
    public HashSet<string> DisabledAgents { get; set; } = [];
    public bool StartWithWindows { get; set; }

    [JsonIgnore]
    public IslandAppearance AppearanceMode => Enum.TryParse<IslandAppearance>(Appearance, out var a) ? a : IslandAppearance.System;
    [JsonIgnore]
    public DockEdge Edge => Enum.TryParse<Core.DockEdge>(DockEdge, out var e) ? e : Core.DockEdge.Top;
    [JsonIgnore]
    public NotificationPolicy Policy => Enum.TryParse<Core.NotificationPolicy>(NotificationPolicy, out var p) ? p : Core.NotificationPolicy.Standard;

    public EngineConfig EngineConfig() => new EngineConfig
    {
        SampleInterval = SampleInterval,
        CpuThreshold = CpuThreshold,
        TokenAlertEnabled = TokenAlertEnabled,
        TokenAlertThreshold = TokenAlertThreshold,
    }.Normalized();

    private static string Dir => Path.Combine(
        Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData), "AgentIsland");
    private static string FilePath => Path.Combine(Dir, "settings.json");

    private static readonly JsonSerializerOptions JsonOpts = new()
    {
        WriteIndented = true,
        Encoder = System.Text.Encodings.Web.JavaScriptEncoder.UnsafeRelaxedJsonEscaping,
    };

    public void Save()
    {
        try
        {
            Directory.CreateDirectory(Dir);
            File.WriteAllText(FilePath, JsonSerializer.Serialize(this, JsonOpts));
        }
        catch (IOException) { /* 写盘失败不静默成功，但也不崩 UI */ }
        catch (UnauthorizedAccessException) { }
    }

    public static SettingsStore Load()
    {
        try
        {
            if (File.Exists(FilePath))
            {
                var loaded = JsonSerializer.Deserialize<SettingsStore>(File.ReadAllText(FilePath), JsonOpts);
                if (loaded != null) return loaded;
            }
        }
        catch (IOException) { }
        catch (JsonException) { }
        return new SettingsStore();
    }
}
