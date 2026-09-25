using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using AgentIsland.Core;

namespace AgentIsland.UI;

/// 分栏设置窗口（对应 macOS SettingsView 的「通用与外观 / Agent 监控 / 引擎与性能 / 关于」）
public sealed class SettingsWindow : Window
{
    private static Theme T => ThemeManager.Current;

    public SettingsWindow(SettingsStore settings)
    {
        Title = "AgentIsland 偏好设置";
        Width = 560;
        Height = 460;
        WindowStartupLocation = WindowStartupLocation.CenterScreen;
        Background = new SolidColorBrush(T.Canvas);
        ResizeMode = ResizeMode.NoResize;

        Content = BuildTabs(settings);
        ThemeManager.ThemeChanged += OnThemeChanged;
        Closed += (_, _) => ThemeManager.ThemeChanged -= OnThemeChanged;
    }

    private void OnThemeChanged()
    {
        Background = new SolidColorBrush(T.Canvas);
        Content = BuildTabs(SettingsStore.Instance);
    }

    private FrameworkElement BuildTabs(SettingsStore s)
    {
        var tabs = new TabControl
        {
            Background = new SolidColorBrush(Colors.Transparent),
            BorderThickness = new Thickness(0),
            Margin = new Thickness(10),
        };

        tabs.Items.Add(MakeTab("通用与外观", BuildGeneral(s)));
        tabs.Items.Add(MakeTab("Agent 监控", BuildAgents(s)));
        tabs.Items.Add(MakeTab("引擎与性能", BuildEngine(s)));
        tabs.Items.Add(MakeTab("关于", BuildAbout()));

        return tabs;
    }

    private static TabItem MakeTab(string header, FrameworkElement content)
    {
        return new TabItem
        {
            Header = header,
            Content = new ScrollViewer
            {
                VerticalScrollBarVisibility = ScrollBarVisibility.Auto,
                Content = content,
                Margin = new Thickness(4),
            },
        };
    }

    private FrameworkElement BuildGeneral(SettingsStore s)
    {
        var panel = new StackPanel { Margin = new Thickness(12) };

        panel.Children.Add(MakeLabel("外观主题"));
        panel.Children.Add(MakeCombo(
            [("system", "跟随系统"), ("light", "浅色"), ("dark", "深色")],
            s.Appearance,
            v => { s.Appearance = v; s.Save(); ThemeManager.Apply(Enum.Parse<IslandAppearance>(v)); }));

        panel.Children.Add(MakeLabel("贴边位置"));
        var edgeWrap = new WrapPanel();
        foreach (DockEdge edge in Enum.GetValues<DockEdge>())
        {
            var chip = MakeToggleChip(edge.Label(), s.Edge == edge, () =>
            {
                s.DockEdge = edge.ToString();
                s.Save();
                IslandWindowChanged?.Invoke(edge);
            });
            edgeWrap.Children.Add(chip);
        }
        panel.Children.Add(edgeWrap);

        panel.Children.Add(MakeLabel("光标离开后自动收起延迟（秒）"));
        panel.Children.Add(MakeSlider(0.2, 5, s.CollapseDelay, "0.0",
            v => { s.CollapseDelay = Math.Round(v, 1); s.Save(); CollapseDelayChanged?.Invoke(Math.Round(v, 1)); }));

        panel.Children.Add(MakeLabel("通知模式"));
        panel.Children.Add(MakeCombo(
            [("standard", "标准模式"), ("focus", "专注免打扰"), ("silent", "完全静默")],
            s.NotificationPolicy,
            v => { s.NotificationPolicy = v; s.Save(); }));

        panel.Children.Add(MakeCheck("播放提示音", s.PlayCompletionSound, v => { s.PlayCompletionSound = v; s.Save(); }));

        return panel;
    }

    private FrameworkElement BuildAgents(SettingsStore s)
    {
        var panel = new StackPanel { Margin = new Thickness(12) };
        panel.Children.Add(MakeLabel("启用监控的 Agent（未启用的不再出现在岛上与 CLI）"));
        foreach (var profile in AgentRegistry.Builtin)
        {
            var enabled = !s.DisabledAgents.Contains(profile.Id);
            panel.Children.Add(MakeCheck($"{profile.Emoji} {profile.Name}", enabled, v =>
            {
                if (v) s.DisabledAgents.Remove(profile.Id);
                else s.DisabledAgents.Add(profile.Id);
                s.Save();
            }));
        }
        panel.Children.Add(MakeLabel("会话目录与 Token 明细源随档案声明；新增 Agent 请修改 AgentRegistry.cs"));
        return panel;
    }

    private FrameworkElement BuildEngine(SettingsStore s)
    {
        var panel = new StackPanel { Margin = new Thickness(12) };

        panel.Children.Add(MakeLabel("采样间隔：有活动时（秒，0.5–600）"));
        panel.Children.Add(MakeSlider(0.5, 10, s.SampleInterval, "0.0",
            v => { s.SampleInterval = Math.Round(v, 1); s.Save(); }));

        panel.Children.Add(MakeLabel("CPU 工作判定阈值（%，1–50）"));
        panel.Children.Add(MakeSlider(1, 50, s.CpuThreshold, "0",
            v => { s.CpuThreshold = Math.Round(v, 0); s.Save(); }));

        panel.Children.Add(MakeCheck("Token 消耗突增告警", s.TokenAlertEnabled, v => { s.TokenAlertEnabled = v; s.Save(); }));
        panel.Children.Add(MakeLabel($"Token 告警阈值（{s.TokenAlertThreshold:N0} tokens）"));
        panel.Children.Add(MakeSlider(1000, 2_000_000, s.TokenAlertThreshold, "N0",
            v => { s.TokenAlertThreshold = (int)v; s.Save(); }));

        panel.Children.Add(MakeLabel("本地 Webhook：127.0.0.1:41999（POST /notify、/event 直推事件；/session 需令牌）"));
        return panel;
    }

    private FrameworkElement BuildAbout()
    {
        var panel = new StackPanel { Margin = new Thickness(12) };
        panel.Children.Add(MakeLabel("AgentIsland for Windows"));
        panel.Children.Add(new TextBlock
        {
            Text = "Agent 会话灵动岛监控器 — Windows 端 v0.1.0\n\n" +
                   "五态状态机（working / attention / completed / idle / offline）、\n" +
                   "四边贴边 6pt 微细条 + 弹性展开、Token 用量统计、本地 Webhook。\n\n" +
                   "与 macOS 端共用同一套设计令牌与判定口径。",
            FontSize = 12,
            Margin = new Thickness(0, 6, 0, 0),
            LineHeight = 20,
        });
        return panel;
    }

    // MARK: 小控件工厂

    private static event Action<DockEdge>? IslandWindowChanged;
    private static event Action<double>? CollapseDelayChanged;

    /// 设置窗口把静态事件接到运行中的窗口（由 App 组装）
    public static void Wire(IslandWindow island)
    {
        IslandWindowChanged += edge => island.SetDockEdge(edge);
        CollapseDelayChanged += delay => island.SetCollapseDelay(delay);
    }

    private TextBlock MakeLabel(string text) => new()
    {
        Text = text,
        FontSize = 12,
        FontWeight = FontWeights.SemiBold,
        Foreground = new SolidColorBrush(T.OnDark),
        Margin = new Thickness(0, 14, 0, 6),
    };

    private ComboBox MakeCombo((string value, string label)[] items, string current, Action<string> onChange)
    {
        var combo = new ComboBox { Width = 200, HorizontalAlignment = HorizontalAlignment.Left };
        foreach (var (value, label) in items)
        {
            var item = new ComboBoxItem { Content = label, Tag = value };
            combo.Items.Add(item);
            if (value == current) combo.SelectedItem = item;
        }
        combo.SelectionChanged += (_, _) =>
        {
            if (combo.SelectedItem is ComboBoxItem { Tag: { } tag }) onChange(tag.ToString()!);
        };
        return combo;
    }

    private FrameworkElement MakeSlider(double min, double max, double value, string format, Action<double> onChange)
    {
        var slider = new Slider
        {
            Minimum = min,
            Maximum = max,
            Value = value,
            TickFrequency = (max - min) / 40,
            IsSnapToTickEnabled = false,
            Width = 300,
            HorizontalAlignment = HorizontalAlignment.Left,
        };
        var valueText = new TextBlock
        {
            FontSize = 11,
            Foreground = new SolidColorBrush(T.OnDarkMuted),
            VerticalAlignment = VerticalAlignment.Center,
            Margin = new Thickness(8, 0, 0, 0),
            Text = value.ToString(format),
        };
        slider.ValueChanged += (_, _) => { valueText.Text = slider.Value.ToString(format); onChange(slider.Value); };
        var panel = new StackPanel { Orientation = Orientation.Horizontal };
        panel.Children.Add(slider);
        panel.Children.Add(valueText);
        return panel;
    }

    private CheckBox MakeCheck(string label, bool isChecked, Action<bool> onChange)
    {
        var cb = new CheckBox
        {
            Content = label,
            IsChecked = isChecked,
            FontSize = 12,
            Foreground = new SolidColorBrush(T.OnDark),
            Margin = new Thickness(0, 4, 0, 4),
        };
        cb.Checked += (_, _) => onChange(true);
        cb.Unchecked += (_, _) => onChange(false);
        return cb;
    }

    private Border MakeToggleChip(string label, bool selected, Action onClick)
    {
        var chip = new Border
        {
            CornerRadius = new CornerRadius(7),
            Padding = new Thickness(12, 5, 12, 6),
            Margin = new Thickness(0, 0, 8, 8),
            Background = new SolidColorBrush(selected ? T.SydedockCyan : T.ChipFill),
            BorderBrush = new SolidColorBrush(T.Hairline),
            BorderThickness = new Thickness(0.5),
            Child = new TextBlock
            {
                Text = label,
                FontSize = 11.5,
                Foreground = new SolidColorBrush(selected ? Colors.Black : T.OnDark),
                FontWeight = FontWeights.SemiBold,
            },
            Cursor = System.Windows.Input.Cursors.Hand,
        };
        chip.MouseLeftButtonUp += (_, _) => onClick();
        return chip;
    }
}
