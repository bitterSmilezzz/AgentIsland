using System.Threading;
using System.Windows;
using System.Windows.Threading;
using AgentIsland.Core;
using AgentIsland.UI;

namespace AgentIsland;

public partial class App : Application
{
    private static Mutex? _singleInstance;
    private static IslandWindow? _island;
    private static TrayIcon? _tray;
    private static LocalEventServer? _server;

    public static ActivityEngine Engine { get; private set; } = null!;
    public static SettingsStore Settings { get; private set; } = null!;

    protected override void OnStartup(StartupEventArgs e)
    {
        base.OnStartup(e);

        _singleInstance = new Mutex(true, "AgentIsland.SingleInstance", out var isNew);
        if (!isNew)
        {
            Shutdown(0);
            return;
        }

        DispatcherUnhandledException += (_, args) =>
        {
            // 岛是常驻界面：单点异常记日志但不要把整个应用拖死
            System.IO.File.AppendAllText(
                System.IO.Path.Combine(System.IO.Path.GetTempPath(), "agentisland.log"),
                $"{DateTime.Now:HH:mm:ss} {args.Exception}\n");
            args.Handled = true;
        };

        Settings = SettingsStore.Load();
        ThemeManager.Apply(Settings.AppearanceMode);

        Engine = new ActivityEngine(Settings);
        Engine.Start();

        _server = new LocalEventServer(Engine);
        _server.Start();
        LocalEventServer.EnsureToken();

        _island = new IslandWindow(Engine, Settings);
        _island.Show();

        SettingsWindow.Wire(_island);
        _tray = new TrayIcon(_island, Engine, Settings, OpenSettings);

        // 验收辅助：--expand 启动后自动展开卡片；--demo 注入演示数据；
        // --route=<r> 展开后直接导航（tokenAnalytics / agentDetail:<id>）（自动化截图用）
        if (e.Args.Contains("--demo")) Engine.DemoMode = true;
        var routeArg = e.Args.FirstOrDefault(a => a.StartsWith("--route="));
        if (e.Args.Contains("--expand"))
        {
            var timer = new DispatcherTimer { Interval = TimeSpan.FromSeconds(2) };
            timer.Tick += (_, _) =>
            {
                timer.Stop();
                _island.Expand();
                if (routeArg != null)
                {
                    var route = routeArg["--route=".Length..];
                    if (route.StartsWith("agentDetail:"))
                        _island.Navigate("agentDetail", route["agentDetail:".Length..]);
                    else
                        _island.Navigate(route);
                }
            };
            timer.Start();
        }
    }

    private Window? _settingsWin;

    private Window OpenSettings()
    {
        if (_settingsWin == null || !_settingsWin.IsLoaded)
        {
            _settingsWin = new SettingsWindow(Settings);
            _settingsWin.Show();
            _settingsWin.Activate();
        }
        else
        {
            _settingsWin.Activate();
        }
        return _settingsWin;
    }

    public static void RequestShutdown()
    {
        _server?.Dispose();
        Engine?.Dispose();
        _tray?.Dispose();
        _island?.Close();
        Current?.Shutdown();
    }

    protected override void OnExit(ExitEventArgs e)
    {
        Settings?.Save();
        base.OnExit(e);
    }
}
