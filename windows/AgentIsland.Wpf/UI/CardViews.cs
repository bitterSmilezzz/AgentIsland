using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Media.Animation;
using AgentIsland.Core;

namespace AgentIsland.UI;

// MARK: - 展开卡片构建器（IslandView 的 Windows 对应物）
// 顶栏（状态摘要 + 快捷按钮）→ 活动环看板 → 事件横幅 → 搜索 → Agent 列表 → Token 汇总栏
// 卡内导航：主列表 → agent 详情 / Token 分析 / 实时流水（SubViews）。

internal sealed class CardBuilder
{
    private readonly IslandWindow _win;
    private readonly ActivityEngine _engine;
    private readonly SettingsStore _settings;
    private readonly NavigateHandler _navigate;
    private readonly Action _collapse;

    private static Theme T => ThemeManager.Current;

    public bool EventExpanded { get; set; }

    private string _searchText = "";
    private bool _searchActive;
    private TextBox? _searchBox;

    public CardBuilder(IslandWindow win, ActivityEngine engine, SettingsStore settings,
        NavigateHandler navigate, Action collapse)
    {
        _win = win;
        _engine = engine;
        _settings = settings;
        _navigate = navigate;
        _collapse = collapse;
    }

    // MARK: 布局小工厂

    private static StackPanel HStack(double spacing, params FrameworkElement[] children)
    {
        var p = new StackPanel { Orientation = Orientation.Horizontal };
        for (var i = 0; i < children.Length; i++)
        {
            if (i > 0) children[i].Margin = new Thickness(spacing, children[i].Margin.Top, 0, children[i].Margin.Bottom);
            p.Children.Add(children[i]);
        }
        return p;
    }

    private static StackPanel VStack(double spacing, params FrameworkElement[] children)
    {
        var p = new StackPanel();
        for (var i = 0; i < children.Length; i++)
        {
            if (i > 0) children[i].Margin = new Thickness(children[i].Margin.Left, spacing, 0, 0);
            p.Children.Add(children[i]);
        }
        return p;
    }

    // MARK: 路由分发

    public FrameworkElement Build(string route, string? agentId, (string, string)? sessions) => route switch
    {
        "tokenAnalytics" => BuildShell(SubViews.BuildTokenAnalytics(_engine, () => _navigate("list"))),
        "agentDetail" => BuildShell(SubViews.BuildAgentDetail(_engine, agentId ?? "", () => _navigate("list"), (a, m) => _navigate("sessions", a, (a, m)))),
        "sessions" => BuildShell(SubViews.BuildSessions(_engine, agentId ?? "", sessions?.Item2 ?? "", () => _navigate("agentDetail", agentId))),
        "liveStream" => BuildShell(SubViews.BuildLiveStream(_engine, agentId, () => _navigate("list"))),
        _ => BuildShell(BuildListCard()),
    };

    public void PlayOpenAnimation()
    {
        // 展开瞬间的入场转场由窗口尺寸弹簧承担；此处保留扩展点
    }

    // MARK: 玻璃卡壳（反向倒角 + 渐变底 + 晶莹微反光边）

    internal static FrameworkElement BuildShell(FrameworkElement content, DockEdge edge,
        double cornerRadius = IslandMetrics.RadiusLg)
    {
        var t = T;
        var root = new Grid();

        var shape = new System.Windows.Shapes.Path { Stretch = Stretch.None };
        root.SizeChanged += (_, e) =>
        {
            shape.Data = SideNotchGeometry.Build(e.NewSize.Width, e.NewSize.Height, edge, cornerRadius, IslandMetrics.NotchInset);
        };

        shape.Fill = new LinearGradientBrush
        {
            StartPoint = new Point(0, 0),
            EndPoint = new Point(0, 1),
            GradientStops = { new GradientStop(t.GlassTop, 0), new GradientStop(t.GlassBottom, 1) },
        };
        shape.Stroke = new LinearGradientBrush
        {
            StartPoint = new Point(0, 0),
            EndPoint = new Point(0, 1),
            GradientStops = { new GradientStop(t.GlassSpecularTop, 0), new GradientStop(t.GlassSpecularBottom, 1) },
        };
        shape.StrokeThickness = 0.75;

        root.Children.Add(shape);
        var host = new ContentControl
        {
            Content = content,
            Margin = new Thickness(IslandMetrics.NotchInset),
            ClipToBounds = true,
        };
        root.Children.Add(host);
        return root;
    }

    private FrameworkElement BuildShell(FrameworkElement content) =>
        BuildShell(content, _win.CurrentEdge);

    // MARK: 主列表卡

    private FrameworkElement BuildListCard()
    {
        var grid = new Grid();
        var row = 0;
        void AddRow(UIElement el)
        {
            Grid.SetRow(el, row++);
            grid.Children.Add(el);
        }
        void AddRowDef() => grid.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });

        AddRowDef();
        AddRow(BuildHeader());
        grid.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        AddRow(Controls.DividerRect());

        var shelf = BuildRingsShelf();
        if (shelf != null)
        {
            AddRowDef();
            AddRow(shelf);
            grid.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
            AddRow(Controls.DividerRect());
        }

        var ev = _engine.LatestEvent;
        if (ev != null)
        {
            AddRowDef();
            AddRow(BuildEventBanner(ev));
            grid.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
            AddRow(Controls.DividerRect());
        }

        if (_searchActive)
        {
            AddRowDef();
            AddRow(BuildSearchBar());
        }

        grid.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });
        var list = BuildList();
        Grid.SetRow(list, row++);
        grid.Children.Add(list);

        var summary = BuildSummaryBar();
        if (summary != null)
        {
            grid.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
            grid.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
            Grid.SetRow(Controls.DividerRect(), row++);
            var div = Controls.DividerRect();
            Grid.SetRow(div, row - 1);
            grid.Children.Add(div);
            Grid.SetRow(summary, row);
            grid.Children.Add(summary);
        }

        grid.Loaded += (_, _) =>
        {
            if (_searchActive && _searchBox != null && !_searchBox.IsKeyboardFocused) _searchBox.Focus();
        };
        return grid;
    }

    // MARK: 顶栏

    private (string title, string? subtitle, string? badge, Color tint, string? subtitleIcon) HeaderPresentation()
    {
        var engine = _engine;
        var t = T;

        if (engine.LatestEvent is { } ev && ev.Type is AgentTaskEvent.EventType.CostSpike or AgentTaskEvent.EventType.Attention)
        {
            var tint = ev.Type == AgentTaskEvent.EventType.CostSpike ? t.DangerRed : t.WarningOrange;
            return (ev.AgentName, ev.SummaryText,
                ev.ExternallyDelivered
                    ? ev.Type == AgentTaskEvent.EventType.CostSpike ? "外部告警" : "外部确认"
                    : ev.Type == AgentTaskEvent.EventType.CostSpike ? "告警" : "待确认",
                tint, ev.Type == AgentTaskEvent.EventType.CostSpike ? "\uE7BA" : "\uE8AF");
        }

        var waiting = engine.VisibleSnapshots.FirstOrDefault(s => s.Level == ActivityLevel.Attention);
        if (waiting != null)
        {
            var action = waiting.CurrentAction ?? "等待你确认";
            return (waiting.Profile.Name, action, "待确认", t.WarningOrange, "\uE8AF");
        }

        var active = engine.VisibleSnapshots.FirstOrDefault(s =>
            s.Level == ActivityLevel.Working && !string.IsNullOrEmpty(s.CurrentAction));
        if (active != null)
        {
            var others = Math.Max(engine.VisibleSnapshots.Count(s => s.Level == ActivityLevel.Working) - 1, 0);
            return (active.Profile.Name, active.CurrentAction, others > 0 ? $"+{others}" : "工作中",
                t.StatusWorking, "\uE756");
        }

        var completedCount = engine.VisibleSnapshots.Count(s => s.Level == ActivityLevel.Completed);
        var workingCount = engine.VisibleSnapshots.Count(s => s.Level == ActivityLevel.Working);
        var statusText = workingCount > 0 ? $"{workingCount} 个 Agent 正在工作"
            : completedCount > 0 ? $"{completedCount} 个任务已完成" : "全部 Agent 待机";
        return (statusText, null, null, workingCount > 0 ? t.StatusWorking : t.OnDarkMuted, null);
    }

    private FrameworkElement BuildHeader()
    {
        var t = T;
        var (title, subtitle, badge, tint, subtitleIcon) = HeaderPresentation();

        var grid = new Grid
        {
            Margin = new Thickness(IslandMetrics.PageMargin, IslandMetrics.HeaderPaddingTop, IslandMetrics.PageMargin, IslandMetrics.HeaderPaddingBottom),
        };

        var stack = new StackPanel();
        var dotHost = new Grid { Width = 12, Height = 12, VerticalAlignment = VerticalAlignment.Center };
        var dot = new System.Windows.Shapes.Ellipse
        {
            Width = 9,
            Height = 9,
            Fill = new SolidColorBrush(tint),
            VerticalAlignment = VerticalAlignment.Center,
            HorizontalAlignment = HorizontalAlignment.Center,
        };
        dotHost.Children.Add(dot);
        if (_engine.AnyWorking)
        {
            var pulse = new System.Windows.Shapes.Ellipse
            {
                Width = 9,
                Height = 9,
                Fill = new SolidColorBrush(tint),
                Opacity = 0.35,
                VerticalAlignment = VerticalAlignment.Center,
                HorizontalAlignment = HorizontalAlignment.Center,
            };
            dotHost.Children.Add(pulse);
            pulse.BeginAnimation(FrameworkElement.WidthProperty, new DoubleAnimation(9, 16, TimeSpan.FromSeconds(1.1)) { RepeatBehavior = RepeatBehavior.Forever });
            pulse.BeginAnimation(FrameworkElement.HeightProperty, new DoubleAnimation(9, 16, TimeSpan.FromSeconds(1.1)) { RepeatBehavior = RepeatBehavior.Forever });
            pulse.BeginAnimation(UIElement.OpacityProperty, new DoubleAnimation(0.35, 0, TimeSpan.FromSeconds(1.1)) { RepeatBehavior = RepeatBehavior.Forever });
        }

        var titleText = Controls.Text(title, 13, T.OnDark, FontWeights.SemiBold);
        titleText.Margin = new Thickness(4, 0, 0, 0);
        titleText.MaxWidth = 150;
        titleText.TextTrimming = TextTrimming.CharacterEllipsis;
        titleText.VerticalAlignment = VerticalAlignment.Center;

        var line1Children = new List<FrameworkElement> { dotHost, titleText };
        if (badge != null)
        {
            var badgeBorder = new Border
            {
                CornerRadius = new CornerRadius(4),
                Padding = new Thickness(5, 1.5, 5, 2),
                Background = new SolidColorBrush(Color.FromArgb(30, tint.R, tint.G, tint.B)),
                BorderBrush = new SolidColorBrush(Color.FromArgb(80, tint.R, tint.G, tint.B)),
                BorderThickness = new Thickness(0.5),
                VerticalAlignment = VerticalAlignment.Center,
                Child = Controls.Text(badge, 9, tint, FontWeights.SemiBold),
            };
            line1Children.Add(badgeBorder);
        }
        stack.Children.Add(HStack(4, line1Children.ToArray()));

        if (subtitle != null)
        {
            var sub = Controls.Text(subtitle, 10.5, tint);
            sub.MaxWidth = 185;
            sub.TextTrimming = TextTrimming.CharacterEllipsis;
            var line2Children = new List<FrameworkElement>();
            if (subtitleIcon != null) line2Children.Add(Controls.IconText(subtitleIcon, 9.5, tint));
            line2Children.Add(sub);
            var line2 = HStack(4, line2Children.ToArray());
            line2.Margin = new Thickness(13, 1, 0, 0);
            stack.Children.Add(line2);
        }
        grid.Children.Add(stack);

        // 右侧：计数 + 快捷图标
        var right = new StackPanel
        {
            Orientation = Orientation.Horizontal,
            HorizontalAlignment = HorizontalAlignment.Right,
            VerticalAlignment = subtitle != null ? VerticalAlignment.Top : VerticalAlignment.Center,
            Margin = new Thickness(0, subtitle != null ? 2 : 0, 0, 0),
        };
        var visible = _engine.VisibleSnapshots.Length;
        var total = _engine.Snapshots.Length;
        var countText = Controls.Text($"{visible}", 11, T.OnDarkFaint, vertical: VerticalAlignment.Center);
        countText.Margin = new Thickness(0, 0, 6, 0);
        right.Children.Add(countText);

        var collapseGlyph = _win.CurrentEdge switch
        {
            DockEdge.Top => "\uE70E",
            DockEdge.Bottom => "\uE70D",
            DockEdge.Left => "\uE76B",
            _ => "\uE76C",
        };
        var themeGlyph = T.IsDark ? "\uE708" : "\uE706"; // 月 / 日
        right.Children.Add(Controls.IconButton("\uE721", "即时搜索过滤 (/)", () =>
        {
            _searchActive = !_searchActive;
            if (!_searchActive) _searchText = "";
            RebuildList();
        }));
        right.Children.Add(Controls.IconButton(themeGlyph, "外观主题", () =>
            _win.ApplyAppearance(T.IsDark ? IslandAppearance.Light : IslandAppearance.Dark)));
        right.Children.Add(Controls.IconButton("\uE90F", "智能体维护工作台", () => _navigate("liveStream", null)));
        right.Children.Add(Controls.IconButton("\uE81C", "最近事件", () => _navigate("liveStream", _engine.LatestEvent?.AgentId)));
        right.Children.Add(Controls.IconButton(collapseGlyph, "收起灵动岛", _collapse));
        grid.Children.Add(right);

        // 顶栏按住拖拽 + 松手吸附
        grid.MouseLeftButtonDown += (_, e) =>
        {
            if (e.ClickCount == 1)
            {
                try { _win.DragMove(); } catch { }
                _win.SnapToNearestEdge();
            }
        };
        return grid;
    }

    // MARK: 活动环看板（Quick Rings Shelf）

    private FrameworkElement? BuildRingsShelf()
    {
        var snaps = _engine.RingShelfSnapshots;
        if (snaps.Length == 0) return null;
        var t = T;
        var scroll = new ScrollViewer
        {
            HorizontalScrollBarVisibility = ScrollBarVisibility.Hidden,
            VerticalScrollBarVisibility = ScrollBarVisibility.Disabled,
            Padding = new Thickness(IslandMetrics.PageMargin, 5, IslandMetrics.PageMargin, 5),
        };
        var panel = new StackPanel { Orientation = Orientation.Horizontal };
        foreach (var snap in snaps)
        {
            var ring = new AgentRing { Width = 24, Height = 24, VerticalAlignment = VerticalAlignment.Center };
            ring.Update(snap);

            var sub = snap.Level == ActivityLevel.Working ? "工作中"
                : snap.Level == ActivityLevel.Attention ? "等待你确认"
                : snap.Level == ActivityLevel.Completed ? "任务已完成"
                : snap.TokenUsage is { Tokens24h: > 0 } u ? TokenUsage.Compact(u.Tokens24h)
                : snap.Level.Label();
            var subColor = snap.Level == ActivityLevel.Attention ? T.WarningOrange
                : snap.Level is ActivityLevel.Working or ActivityLevel.Completed ? T.RingGreen
                : T.OnDarkFaint;

            var nameText = Controls.Text(snap.Profile.Name, 10, T.OnDark, FontWeights.SemiBold);
            var subText = Controls.Text(sub, 8.5, subColor);
            var chip = new Border
            {
                CornerRadius = new CornerRadius(7),
                Padding = new Thickness(6, 3, 6, 3),
                Margin = new Thickness(0, 0, 8, 0),
                Background = new SolidColorBrush(snap.Level is ActivityLevel.Working or ActivityLevel.Attention ? T.HoverFill : T.CardFill),
                BorderBrush = new SolidColorBrush(snap.Level == ActivityLevel.Attention
                    ? Color.FromArgb(115, t.WarningOrange.R, t.WarningOrange.G, t.WarningOrange.B)
                    : snap.Level == ActivityLevel.Working ? Color.FromArgb(77, t.SydedockEmerald.R, t.SydedockEmerald.G, t.SydedockEmerald.B)
                    : t.Hairline),
                BorderThickness = new Thickness(0.5),
                Child = HStack(6, ring, VStack(1, nameText, subText)),
                Cursor = Cursors.Hand,
            };
            var id = snap.Profile.Id;
            chip.MouseLeftButtonUp += (_, _) => _navigate("agentDetail", id);
            panel.Children.Add(chip);
        }
        scroll.Content = panel;
        return scroll;
    }

    // MARK: 事件横幅

    private FrameworkElement BuildEventBanner(AgentTaskEvent ev)
    {
        var t = T;
        var tint = ev.Type switch
        {
            AgentTaskEvent.EventType.CostSpike => t.DangerRed,
            AgentTaskEvent.EventType.Attention => t.WarningOrange,
            _ => t.StatusWorking,
        };

        var stack = new StackPanel
        {
            Margin = new Thickness(IslandMetrics.PageMargin, 8, IslandMetrics.PageMargin, 8),
            MinHeight = IslandMetrics.EventBannerCollapsedHeight - 16,
        };

        var title = Controls.Text($"{(ev.Type == AgentTaskEvent.EventType.Completed ? "已完成" : "需要确认")}: {ev.SummaryText}", 12, tint, FontWeights.Bold);
        title.MaxWidth = 165;
        title.TextTrimming = TextTrimming.CharacterEllipsis;
        title.VerticalAlignment = VerticalAlignment.Center;

        var reasons = new Border
        {
            CornerRadius = new CornerRadius(5),
            Background = new SolidColorBrush(T.CardFill),
            Padding = new Thickness(8, 2, 8, 3),
            VerticalAlignment = VerticalAlignment.Center,
            Child = Controls.Text(EventExpanded ? "原因 ˄" : "原因 ˅", 9.5, T.OnDarkMuted),
            Cursor = Cursors.Hand,
        };
        reasons.MouseLeftButtonUp += (_, _) =>
        {
            EventExpanded = !EventExpanded;
            RebuildList();
        };

        var close = Controls.IconButton("\uE711", "关闭提醒", DismissCurrentEvent, size: 9, box: 20);
        close.VerticalAlignment = VerticalAlignment.Center;

        var line1 = HStack(6, Controls.IconText("\uE7BA", 11, tint), title, reasons, close);
        stack.Children.Add(line1);

        if (EventExpanded && !string.IsNullOrEmpty(ev.Detail))
        {
            var detailText = Controls.Text(ev.Detail, 9.5, T.OnDarkMuted);
            detailText.TextWrapping = TextWrapping.Wrap;
            detailText.Margin = new Thickness(17, 4, 0, 0);
            stack.Children.Add(detailText);
        }

        var jump = new Border
        {
            CornerRadius = new CornerRadius(5),
            Background = new SolidColorBrush(T.CardFill),
            BorderBrush = new SolidColorBrush(T.Hairline),
            BorderThickness = new Thickness(0.5),
            Padding = new Thickness(10, 3, 10, 4),
            Child = HStack(4, Controls.IconText("\uE8A7", 9, T.OnDarkMuted), Controls.Text("直达", 10, T.OnDarkMuted)),
            Cursor = Cursors.Hand,
            ToolTip = "置顶并激活该智能体窗口/终端",
        };
        var agentId = ev.AgentId;
        jump.MouseLeftButtonUp += (_, _) => AppActivator.ActivateAgent(_engine, agentId);

        var actions = new StackPanel { Orientation = Orientation.Horizontal, HorizontalAlignment = HorizontalAlignment.Right, Margin = new Thickness(0, 6, 0, 0) };
        actions.Children.Add(jump);
        stack.Children.Add(actions);

        var grid = new Grid { Background = new SolidColorBrush(Color.FromArgb(26, tint.R, tint.G, tint.B)) };
        grid.Children.Add(stack);

        // 完成事件 6 秒后自动收起；attention/告警保持
        if (ev.Type == AgentTaskEvent.EventType.Completed)
        {
            var timer = new System.Windows.Threading.DispatcherTimer { Interval = TimeSpan.FromSeconds(6), Tag = ev.Id };
            timer.Tick += (_, _) =>
            {
                timer.Stop();
                if (_engine.LatestEvent?.Id == ev.Id) DismissCurrentEvent();
            };
            timer.Start();
        }
        return grid;
    }

    private void DismissCurrentEvent()
    {
        _engine.ClearLatestEvent();
        EventExpanded = false;
        RebuildList();
    }

    // MARK: 搜索条

    private FrameworkElement BuildSearchBar()
    {
        var box = new TextBox
        {
            Name = "searchBox",
            Background = null!,
            BorderThickness = new Thickness(0),
            Foreground = new SolidColorBrush(T.OnDark),
            CaretBrush = new SolidColorBrush(T.SydedockCyan),
            FontSize = 11,
            FontFamily = Theme.TextFont,
            Margin = new Thickness(6, 0, 0, 0),
            VerticalContentAlignment = VerticalAlignment.Center,
            MinWidth = 200,
            Text = _searchText,
        };
        box.TextChanged += (_, _) => { _searchText = box.Text; RefreshRowsOnly(); };
        box.KeyDown += (_, e) =>
        {
            if (e.Key == Key.Escape)
            {
                _searchActive = false;
                _searchText = "";
                RebuildList();
            }
        };
        _searchBox = box;

        var outer = new Border
        {
            Margin = new Thickness(10, 4, 10, 2),
            CornerRadius = new CornerRadius(6),
            Background = new SolidColorBrush(T.CardFill),
            BorderBrush = new SolidColorBrush(Color.FromArgb(102, T.SydedockCyan.R, T.SydedockCyan.G, T.SydedockCyan.B)),
            BorderThickness = new Thickness(0.8),
            Padding = new Thickness(8, 5, 8, 5),
            Child = HStack(0, Controls.IconText("\uE721", 10, T.SydedockCyan), box),
        };
        return outer;
    }

    // MARK: Agent 列表

    private FrameworkElement BuildList()
    {
        var engine = _engine;
        var snapshots = engine.VisibleSnapshots;

        if (snapshots.Length == 0)
        {
            var empty = new StackPanel { VerticalAlignment = VerticalAlignment.Center, HorizontalAlignment = HorizontalAlignment.Center };
            var icon = Controls.IconText("\uE76E", 22, T.OnDarkFaint);
            icon.HorizontalAlignment = HorizontalAlignment.Center;
            var text = Controls.Text("没有活跃的 Agent", 12, T.OnDarkFaint);
            text.HorizontalAlignment = HorizontalAlignment.Center;
            empty.Children.Add(icon);
            empty.Children.Add(text);
            return empty;
        }

        var filtered = snapshots;
        if (_searchActive && _searchText.Length > 0)
        {
            filtered = snapshots.Where(s =>
                s.Profile.Name.Contains(_searchText, StringComparison.OrdinalIgnoreCase) ||
                s.Profile.Id.Contains(_searchText, StringComparison.OrdinalIgnoreCase)).ToArray();
            if (filtered.Length == 0)
            {
                var none = new StackPanel { VerticalAlignment = VerticalAlignment.Center, HorizontalAlignment = HorizontalAlignment.Center };
                var icon = Controls.IconText("\uE721", 18, T.OnDarkFaint);
                icon.HorizontalAlignment = HorizontalAlignment.Center;
                var text = Controls.Text($"未找到匹配「{_searchText}」的智能体", 11, T.OnDarkFaint);
                text.HorizontalAlignment = HorizontalAlignment.Center;
                none.Children.Add(icon);
                none.Children.Add(text);
                return none;
            }
        }

        var scroll = new ScrollViewer
        {
            VerticalScrollBarVisibility = ScrollBarVisibility.Hidden,
            Padding = new Thickness(0, IslandMetrics.ListVerticalPadding, 0, IslandMetrics.ListVerticalPadding),
        };
        var stack = new StackPanel();
        foreach (var snap in filtered)
            stack.Children.Add(BuildAgentRow(snap));
        scroll.Content = stack;
        return scroll;
    }

    private FrameworkElement BuildAgentRow(AgentSnapshot snap)
    {
        var t = T;
        var hasActionBar = snap.Level is ActivityLevel.Working or ActivityLevel.Attention &&
                           !string.IsNullOrEmpty(snap.CurrentAction);

        var outer = new Border
        {
            CornerRadius = new CornerRadius(10),
            Margin = new Thickness(6, 0, 6, 2),
            Padding = new Thickness(8, hasActionBar ? 5 : 4, 8, hasActionBar ? 5 : 4),
            Background = Brushes.Transparent,
        };

        var grid = new Grid();
        grid.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        if (hasActionBar) grid.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });

        var line1 = new Grid();
        var left = new StackPanel { Orientation = Orientation.Horizontal, VerticalAlignment = VerticalAlignment.Center };

        var ring = new AgentRing { Width = 26, Height = 26, VerticalAlignment = VerticalAlignment.Center };
        ring.Update(snap);

        var nameText = Controls.Text(snap.Profile.Name, 12.5, T.OnDark, FontWeights.SemiBold);
        nameText.MaxWidth = 165;
        nameText.TextTrimming = TextTrimming.CharacterEllipsis;

        var sub = new StackPanel { Orientation = Orientation.Horizontal, Margin = new Thickness(0, 1, 0, 0) };
        if (snap.TokenUsage is { Tokens24h: > 0 } usage)
        {
            var badgeText = Controls.Text(TokenUsage.Compact(usage.Tokens24h), 9, T.OnDark, FontWeights.SemiBold, Theme.MonoFont);
            badgeText.Margin = new Thickness(3, 0, 0, 0);
            var boltBadge = new Border
            {
                CornerRadius = new CornerRadius(999),
                Padding = new Thickness(5, 1.5, 5, 2),
                Background = new SolidColorBrush(T.IsDark ? Color.FromArgb(20, 255, 255, 255) : Color.FromArgb(217, 255, 255, 255)),
                BorderBrush = new SolidColorBrush(T.Hairline),
                BorderThickness = new Thickness(0.5),
                Child = HStack(0, Controls.IconText("\uE945", 7, T.SydedockCyan), badgeText),
                VerticalAlignment = VerticalAlignment.Center,
            };
            sub.Children.Add(boltBadge);
            var spacer = new FrameworkElement { Height = 0, Width = 5 };
            sub.Children.Add(spacer);
        }
        else
        {
            var last = Controls.Text(snap.LastActivityText, 9.5, T.OnDarkFaint);
            last.Margin = new Thickness(0, 0, 5, 0);
            sub.Children.Add(last);
        }
        var dots = new ActivityDots { Width = 34, Height = 10, VerticalAlignment = VerticalAlignment.Center };
        dots.Update(snap);
        sub.Children.Add(dots);

        var nameCol = new StackPanel { Margin = new Thickness(8, 0, 0, 0), VerticalAlignment = VerticalAlignment.Center };
        nameCol.Children.Add(nameText);
        nameCol.Children.Add(sub);
        left.Children.Add(ring);
        left.Children.Add(nameCol);
        line1.Children.Add(left);

        // 右侧集群：常规 = 内存 chip + 状态药丸；悬停 = 快捷操作
        var right = new StackPanel
        {
            Orientation = Orientation.Horizontal,
            HorizontalAlignment = HorizontalAlignment.Right,
            VerticalAlignment = VerticalAlignment.Center,
        };
        if (snap.ProcessRunning && snap.MemoryBytes > 0)
        {
            var mem = Controls.TokenBadgePill(snap.MemoryText);
            mem.Margin = new Thickness(0, 0, 5, 0);
            mem.ToolTip = $"物理内存驻留集 (RSS): {snap.MemoryText}";
            right.Children.Add(mem);
        }
        if (snap.IsHung == true)
        {
            var hung = new Border
            {
                CornerRadius = new CornerRadius(999),
                Padding = new Thickness(6, 2, 6, 2.5),
                Background = new SolidColorBrush(Color.FromArgb(46, t.DangerRed.R, t.DangerRed.G, t.DangerRed.B)),
                BorderBrush = new SolidColorBrush(Color.FromArgb(102, t.DangerRed.R, t.DangerRed.G, t.DangerRed.B)),
                BorderThickness = new Thickness(0.5),
                ToolTip = "检测到进程持续异常高负荷且缺乏会话响应，疑似处于死循环或线程死锁状态",
                Child = Controls.Text("疑似卡死", 9, T.DangerRed, FontWeights.Bold),
            };
            right.Children.Add(hung);
        }
        else
        {
            right.Children.Add(Controls.StatusPill(snap.Level));
        }
        line1.Children.Add(right);
        Grid.SetRow(line1, 0);
        grid.Children.Add(line1);

        // 工作态实时动作横条
        if (hasActionBar && snap.CurrentAction != null)
        {
            var actionColor = snap.Level == ActivityLevel.Attention ? t.WarningOrange : t.StatusWorking;
            var lightText = snap.Level == ActivityLevel.Attention ? ThemeColors.Amber700 : ThemeColors.Emerald700;
            var textColor = T.IsDark ? actionColor : lightText;
            var barBg = T.IsDark ? Color.FromArgb(20, actionColor.R, actionColor.G, actionColor.B)
                : snap.Level == ActivityLevel.Attention ? ThemeColors.Amber50 : ThemeColors.Emerald50;
            var barBorder = T.IsDark ? Color.FromArgb(56, actionColor.R, actionColor.G, actionColor.B)
                : snap.Level == ActivityLevel.Attention ? ThemeColors.Amber200 : ThemeColors.Emerald200;

            var actionText = Controls.Text(snap.CurrentAction, 9.5, textColor, font: Theme.MonoFont);
            actionText.MaxWidth = 172;
            actionText.TextTrimming = TextTrimming.CharacterEllipsis;
            actionText.VerticalAlignment = VerticalAlignment.Center;

            var barChildren = new List<FrameworkElement>
            {
                Controls.IconText(snap.Level == ActivityLevel.Attention ? "\uE8AF" : "\uE756", 8.5, textColor),
                actionText,
            };
            if (snap.SubagentCount > 0)
            {
                barChildren.Add(new Border
                {
                    CornerRadius = new CornerRadius(999),
                    Padding = new Thickness(4, 1, 4, 1.5),
                    Background = new SolidColorBrush(Color.FromArgb(46, T.SydedockCyan.R, T.SydedockCyan.G, T.SydedockCyan.B)),
                    Child = Controls.Text($"{snap.SubagentCount}子任务", 8, T.SydedockCyan, FontWeights.Bold),
                });
            }
            var bar = new Border
            {
                CornerRadius = new CornerRadius(5),
                Margin = new Thickness(34, 3, 0, 0),
                Padding = new Thickness(7, 3, 7, 3),
                Background = new SolidColorBrush(barBg),
                BorderBrush = new SolidColorBrush(barBorder),
                BorderThickness = new Thickness(0.5),
                Child = HStack(5, barChildren.ToArray()),
                ToolTip = snap.CurrentAction,
            };
            Grid.SetRow(bar, 1);
            grid.Children.Add(bar);
        }

        outer.Child = grid;

        // 悬停反馈 + 快捷操作（渐进披露）
        // (悬停时直接重设 outer.Background)
        outer.Background = Brushes.Transparent;
        Border? actionsHost = null;
        outer.MouseEnter += (_, _) =>
        {
            outer.Background = T.IsDark
                ? new SolidColorBrush(Color.FromArgb(46, 255, 255, 255))
                : new SolidColorBrush(Color.FromArgb(230, 255, 255, 255));
            if (snap.ProcessRunning && actionsHost == null)
            {
                actionsHost = BuildRowQuickActions(snap);
                line1.Children.Add(actionsHost);
                right.Visibility = Visibility.Collapsed;
            }
        };
        outer.MouseLeave += (_, _) =>
        {
            outer.Background = Brushes.Transparent;
            if (actionsHost != null)
            {
                line1.Children.Remove(actionsHost);
                actionsHost = null;
                right.Visibility = Visibility.Visible;
            }
        };
        var id = snap.Profile.Id;
        outer.MouseLeftButtonUp += (_, _) => _navigate("agentDetail", id);

        return outer;
    }

    private Border BuildRowQuickActions(AgentSnapshot snap)
    {
        var kill = Controls.IconButton("\uE711", "终止进程（再次点击确认）", () =>
            _engine.TerminateAgent(snap.Pid, snap.Profile.Id), size: 9, box: 20);
        var stream = Controls.IconButton("\uE756", "实时流水", () => _navigate("liveStream", snap.Profile.Id), size: 9, box: 20);
        var focus = Controls.IconButton("\uE8A7", "直达窗口", () => AppActivator.ActivateAgent(_engine, snap.Profile.Id), size: 9, box: 20);
        kill.Margin = new Thickness(0, 0, 4, 0);
        stream.Margin = new Thickness(0, 0, 4, 0);
        return new Border
        {
            Child = new StackPanel { Orientation = Orientation.Horizontal, Children = { kill, stream, focus } },
            HorizontalAlignment = HorizontalAlignment.Right,
            VerticalAlignment = VerticalAlignment.Center,
        };
    }

    // MARK: Token 汇总栏

    private FrameworkElement? BuildSummaryBar()
    {
        var total = _engine.GrandTotal;
        if (total.TokensTotal <= 0 && total.Tokens24h <= 0) return null;

        var leftChildren = new List<FrameworkElement> { MiniTag("24H", T.SydedockCyan, 3.5) };
        leftChildren.Add(Controls.Text(TokenUsage.Compact(total.Tokens24h), 10.5, T.OnDark, FontWeights.Bold, Theme.MonoFont));
        var cost24 = TokenUsage.Cost(total.Cost24h);
        if (cost24.Length > 0)
            leftChildren.Add(Controls.Text(cost24, 9.5, T.SydedockAmber, FontWeights.SemiBold, Theme.MonoFont));

        var rightChildren = new List<FrameworkElement> { MiniTag("TOTAL", T.OnDarkMuted, 3) };
        rightChildren.Add(Controls.Text(TokenUsage.Compact(total.TokensTotal), 10, T.OnDark, FontWeights.SemiBold, Theme.MonoFont));
        var costTotal = TokenUsage.Cost(total.CostTotal);
        if (costTotal.Length > 0)
            rightChildren.Add(Controls.Text(costTotal, 9.5, T.SydedockAmber, font: Theme.MonoFont));
        rightChildren.Add(Controls.IconText("\uE76C", 8, T.OnDarkFaint));

        var panel = new Grid();
        var left = HStack(5, leftChildren.ToArray());
        var right = HStack(5, rightChildren.ToArray());
        right.HorizontalAlignment = HorizontalAlignment.Right;
        panel.Children.Add(left);
        panel.Children.Add(right);

        var outer = new Border
        {
            Margin = new Thickness(6, 2, 6, 4),
            Padding = new Thickness(8, 3.5, 8, 3.5),
            CornerRadius = new CornerRadius(9),
            Background = new SolidColorBrush(T.CardFill),
            BorderBrush = new SolidColorBrush(T.Hairline),
            BorderThickness = new Thickness(0.5),
            Cursor = Cursors.Hand,
            ToolTip = "打开 Token 时间分析",
            Child = panel,
        };
        outer.MouseLeftButtonUp += (_, _) => _navigate("tokenAnalytics");
        return outer;
    }

    private static Border MiniTag(string text, Color color, double radius)
    {
        return new Border
        {
            CornerRadius = new CornerRadius(radius),
            Padding = new Thickness(4.5, 1.5, 4.5, 2),
            Background = new SolidColorBrush(Color.FromArgb(40, color.R, color.G, color.B)),
            BorderBrush = new SolidColorBrush(Color.FromArgb(90, color.R, color.G, color.B)),
            BorderThickness = new Thickness(0.5),
            VerticalAlignment = VerticalAlignment.Center,
            Child = Controls.Text(text, 8, color, FontWeights.Bold, Theme.MonoFont),
        };
    }

    // MARK: 刷新

    public void RebuildList()
    {
        // 正在搜索输入时跳过重建（保住焦点）
        if (_searchBox != null && _searchBox.IsKeyboardFocused) return;
        _win.RefreshExpandedCard();
    }

    /// 搜索过滤引起的行集变化：同样走整卡重建，但避开焦点判定
    private void RefreshRowsOnly() => _win.RefreshExpandedCard(force: true);

    internal void SetSearchActive(bool active) => _searchActive = active;
}

// MARK: - 窗口激活助手（直达窗口）

internal static class AppActivator
{
    [System.Runtime.InteropServices.DllImport("user32.dll")]
    private static extern bool SetForegroundWindow(nint hWnd);

    [System.Runtime.InteropServices.DllImport("user32.dll")]
    private static extern bool ShowWindow(nint hWnd, int nCmdShow);

    public static void ActivateAgent(ActivityEngine engine, string agentId)
    {
        var snap = engine.VisibleSnapshots.FirstOrDefault(s => s.Profile.Id == agentId);
        if (snap?.Pid is { } pid)
        {
            try
            {
                var proc = System.Diagnostics.Process.GetProcessById(pid);
                if (proc.MainWindowHandle != 0)
                {
                    ShowWindow(proc.MainWindowHandle, 9); // SW_RESTORE
                    SetForegroundWindow(proc.MainWindowHandle);
                }
            }
            catch
            {
                // CLI 进程无窗口或已退出：回退岛内详情即可
            }
        }
    }
}
