using System.Drawing;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Threading;
using AgentIsland.Core;

namespace AgentIsland.UI;

/// 菜单栏常驻图标（MenuBarExtra 的 Windows 对应物：NotifyIcon 托盘）
public sealed class TrayIcon : IDisposable
{
    private readonly System.Windows.Forms.NotifyIcon _icon;
    private readonly IslandWindow _island;
    private readonly ActivityEngine _engine;
    private readonly SettingsStore _settings;
    private readonly Func<Window> _openSettings;
    private readonly DispatcherTimer _statusTimer;

    public TrayIcon(IslandWindow island, ActivityEngine engine, SettingsStore settings, Func<Window> openSettings)
    {
        _island = island;
        _engine = engine;
        _settings = settings;
        _openSettings = openSettings;

        _icon = new System.Windows.Forms.NotifyIcon
        {
            Icon = MakeIcon(),
            Text = "AgentIsland",
            Visible = true,
        };

        var menu = new System.Windows.Forms.ContextMenuStrip();
        RebuildMenu(menu);
        _icon.ContextMenuStrip = menu;
        _icon.MouseClick += (_, e) =>
        {
            if (e.Button == System.Windows.Forms.MouseButtons.Left) _island.Toggle();
        };

        // 状态徽标：每 2s 同步一次提示文案与图标色
        _statusTimer = new DispatcherTimer { Interval = TimeSpan.FromSeconds(2) };
        _statusTimer.Tick += (_, _) =>
        {
            var working = _engine.VisibleSnapshots.Count(s => s.Level == ActivityLevel.Working);
            var attention = _engine.VisibleSnapshots.Count(s => s.Level == ActivityLevel.Attention);
            _icon.Text = attention > 0
                ? $"AgentIsland — {attention} 个待确认"
                : working > 0 ? $"AgentIsland — {working} 个工作中" : "AgentIsland — 全部待机";
            RebuildMenu(menu);
        };
        _statusTimer.Start();
    }

    private void RebuildMenu(System.Windows.Forms.ContextMenuStrip menu)
    {
        menu.Items.Clear();
        menu.Items.Add("展开 / 收起灵动岛", null, (_, _) => _island.Toggle());

        var edgeMenu = new System.Windows.Forms.ToolStripMenuItem("吸附边缘");
        foreach (DockEdge edge in Enum.GetValues<DockEdge>())
        {
            var edgeItem = new System.Windows.Forms.ToolStripMenuItem(edge.Label())
            {
                Checked = _island.CurrentEdge == edge,
            };
            var e2 = edge;
            edgeItem.Click += (_, _) => _island.SetDockEdge(e2);
            edgeMenu.DropDownItems.Add(edgeItem);
        }
        menu.Items.Add(edgeMenu);

        var themeMenu = new System.Windows.Forms.ToolStripMenuItem("外观主题");
        foreach (IslandAppearance mode in Enum.GetValues<IslandAppearance>())
        {
            var label = mode switch
            {
                IslandAppearance.System => "跟随系统",
                IslandAppearance.Light => "浅色",
                IslandAppearance.Dark => "深色",
                _ => mode.ToString(),
            };
            var item = new System.Windows.Forms.ToolStripMenuItem(label)
            {
                Checked = _settings.AppearanceMode == mode,
            };
            var m2 = mode;
            item.Click += (_, _) => _island.ApplyAppearance(m2);
            themeMenu.DropDownItems.Add(item);
        }
        menu.Items.Add(themeMenu);

        menu.Items.Add(new System.Windows.Forms.ToolStripSeparator());
        menu.Items.Add("偏好设置…", null, (_, _) => _openSettings());
        menu.Items.Add(new System.Windows.Forms.ToolStripSeparator());
        menu.Items.Add("退出 AgentIsland", null, (_, _) => App.RequestShutdown());
    }

    private static Icon MakeIcon()
    {
        // 简单的岛形图标：圆角胶囊 + 呼吸点
        using var bmp = new Bitmap(32, 32);
        using var g = Graphics.FromImage(bmp);
        g.SmoothingMode = System.Drawing.Drawing2D.SmoothingMode.AntiAlias;
        using var brush = new SolidBrush(Color.FromArgb(0x16, 0x17, 0x22));
        g.FillRoundedRectangle(brush, new Rectangle(2, 10, 28, 12), 6);
        using var dot = new SolidBrush(Color.FromArgb(0x30, 0xD1, 0x58));
        g.FillEllipse(dot, 6, 13, 6, 6);
        return Icon.FromHandle(bmp.GetHicon());
    }

    public void Dispose()
    {
        _statusTimer.Stop();
        _icon.Visible = false;
        _icon.Dispose();
    }
}

internal static class GraphicsExt
{
    public static void FillRoundedRectangle(this Graphics g, Brush brush, Rectangle rect, float radius)
    {
        using var path = new System.Drawing.Drawing2D.GraphicsPath();
        var d = radius * 2;
        path.AddArc(rect.X, rect.Y, d, d, 180, 90);
        path.AddArc(rect.Right - d, rect.Y, d, d, 270, 90);
        path.AddArc(rect.Right - d, rect.Bottom - d, d, d, 0, 90);
        path.AddArc(rect.X, rect.Bottom - d, d, d, 90, 90);
        path.CloseFigure();
        g.FillPath(brush, path);
    }
}
