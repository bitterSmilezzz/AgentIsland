using System.IO;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Windows.Input;
using System.Windows.Media;
using AgentIsland.Core;

namespace AgentIsland.UI;

// MARK: - 卡内二级/三级页（TokenAnalyticsView / AgentDetailView / SessionListView / LiveLogStreamView）

internal static class SubViews
{
    private static Theme T => ThemeManager.Current;

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
            if (i > 0) children[i].Margin = new Thickness(0, spacing, 0, 0);
            p.Children.Add(children[i]);
        }
        return p;
    }

    private static TextBlock Text(string s, double size, Color color, FontWeight? weight = null, FontFamily? font = null)
    {
        var tb = new TextBlock
        {
            Text = s,
            FontSize = size,
            Foreground = new SolidColorBrush(color),
            VerticalAlignment = VerticalAlignment.Center,
        };
        if (weight != null) tb.FontWeight = weight.Value;
        if (font != null) tb.FontFamily = font;
        return tb;
    }

    private static Border Card(FrameworkElement content, double padding = 10)
    {
        return new Border
        {
            CornerRadius = new CornerRadius(10),
            Background = new SolidColorBrush(T.CardFill),
            BorderBrush = new SolidColorBrush(T.Hairline),
            BorderThickness = new Thickness(0.5),
            Padding = new Thickness(padding),
            Child = content,
        };
    }

    /// 二级页壳：返回按钮 + 标题/副标题 + 内容
    private static FrameworkElement BuildPage(string title, string? subtitle, FrameworkElement content, Action back)
    {
        var grid = new Grid();
        grid.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        grid.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        grid.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });

        var backBtn = new Border
        {
            Width = 24,
            Height = 24,
            CornerRadius = new CornerRadius(12),
            Background = new SolidColorBrush(T.ChipFill),
            Child = new TextBlock
            {
                Text = "\uE76B",
                FontFamily = Theme.IconFont,
                FontSize = 10,
                Foreground = new SolidColorBrush(T.OnDark),
                TextAlignment = TextAlignment.Center,
                VerticalAlignment = VerticalAlignment.Center,
            },
            Cursor = Cursors.Hand,
        };
        backBtn.MouseLeftButtonUp += (_, _) => back();

        var titleCol = VStack(0, Text(title, 12.5, T.OnDark, FontWeights.SemiBold));
        if (subtitle != null) titleCol.Children.Add(Text(subtitle, 9.5, T.OnDarkFaint));
        titleCol.Margin = new Thickness(8, 0, 0, 0);

        var header = HStack(0, backBtn, titleCol);
        header.Margin = new Thickness(IslandMetrics.PageMargin, 10, IslandMetrics.PageMargin, 8);
        Grid.SetRow(header, 0);
        grid.Children.Add(header);

        var div = Controls.DividerRect();
        Grid.SetRow(div, 1);
        grid.Children.Add(div);

        var scroll = new ScrollViewer
        {
            VerticalScrollBarVisibility = ScrollBarVisibility.Hidden,
            Padding = new Thickness(0, 6, 0, 6),
        };
        scroll.Content = content;
        Grid.SetRow(scroll, 2);
        grid.Children.Add(scroll);
        return grid;
    }

    // MARK: Token 用量分析页

    public static FrameworkElement BuildTokenAnalytics(ActivityEngine engine, Action back)
    {
        List<(AgentProfile profile, TokenReport report)> reports;
        if (engine.DemoMode)
        {
            var demo = engine.GetTokenReport("demo")!;
            reports = [(AgentRegistry.Builtin[0], demo)];
        }
        else
        {
            reports = [];
            foreach (var p in AgentRegistry.EnabledProfiles(SettingsStore.Instance))
            {
                var r = engine.GetTokenReport(p.Id);
                if (r != null && (r.Usage.TokensTotal > 0 || r.Usage.Tokens24h > 0))
                    reports.Add((p, r));
            }
        }

        var t24 = reports.Sum(r => r.report.Usage.Tokens24h);
        var tTotal = reports.Sum(r => r.report.Usage.TokensTotal);
        var c24 = reports.Sum(r => r.report.Usage.Cost24h);
        var cTotal = reports.Sum(r => r.report.Usage.CostTotal);
        var hourly = new Dictionary<DateTime, long>();
        foreach (var (_, r) in reports)
            foreach (var h in r.Hourly30d)
                hourly[h.Hour] = hourly.TryGetValue(h.Hour, out var v) ? v + h.Tokens : h.Tokens;

        var content = VStack(8);
        content.Margin = new Thickness(IslandMetrics.PageMargin, 2, IslandMetrics.PageMargin, 8);

        // 24h / 7天 / 30天 分段控件
        var trendChart = new TrendChart(hourly) { Height = 110 };
        content.Children.Add(BuildSegmented(["24h", "7天", "30天"], 0, idx =>
        {
            trendChart.Mode = idx switch
            {
                1 => TrendChart.Range.Days7,
                2 => TrendChart.Range.Days30,
                _ => TrendChart.Range.Hours24,
            };
        }));

        // 月末预测
        var remainingDays = Math.Max(1, DateTime.DaysInMonth(DateTime.Now.Year, DateTime.Now.Month) - DateTime.Now.Day);
        var monthProjection = t24 * remainingDays;
        var projLeft = VStack(2,
            HStack(5, Text("\uE9D9", 11, T.SydedockBlue), Text("月末用量与成本预测", 11.5, T.OnDark, FontWeights.SemiBold)),
            HStack(14,
                VStack(2, Text("预估月末消耗", 9, T.OnDarkFaint), Text(TokenUsage.Compact(monthProjection), 14, T.OnDark, FontWeights.Bold, Theme.MonoFont)),
                VStack(2, Text("预估月末费用", 9, T.OnDarkFaint), Text($"${c24 * remainingDays:0.00}", 14, T.SydedockEmerald, FontWeights.Bold, Theme.MonoFont)),
                VStack(2, Text("当月剩余自然日", 9, T.OnDarkFaint), Text($"{remainingDays} 天", 14, T.OnDark, FontWeights.Bold, Theme.MonoFont))));
        content.Children.Add(Card(projLeft));

        // 总体用量
        var totals = HStack(24,
            VStack(2, Text(TokenUsage.Compact(t24), 17, T.OnDark, FontWeights.Bold, Theme.MonoFont), Text("24h 用量", 9.5, T.OnDarkFaint)),
            VStack(2, Text(c24 > 0.005 ? $"${c24:0.00}" : "—", 17, T.OnDark, FontWeights.Bold, Theme.MonoFont), Text("费用", 9.5, T.OnDarkFaint)),
            VStack(2, Text(TokenUsage.Compact(tTotal), 17, T.OnDark, FontWeights.Bold, Theme.MonoFont), Text("累计", 9.5, T.OnDarkFaint)));
        totals.HorizontalAlignment = HorizontalAlignment.Center;
        content.Children.Add(Card(totals));

        // 使用趋势（折线）
        content.Children.Add(Card(VStack(6, TrendHeader(hourly), trendChart)));

        // 24h 协同节律热力图
        content.Children.Add(Card(BuildHeatmap(hourly)));

        // 按工具用量
        var toolList = VStack(8);
        var maxTokens = Math.Max(1, reports.Max(r => (long?)r.report.Usage.Tokens24h) ?? 1);
        foreach (var (profile, report) in reports.OrderByDescending(r => r.report.Usage.Tokens24h))
        {
            var pct = report.Usage.Tokens24h * 100.0 / Math.Max(1, t24);
            var rowGrid = new Grid();
            var left = HStack(8,
                Controls.IconText(profile.Glyph, 11, T.OnDarkMuted),
                Text(profile.Name, 11.5, T.OnDark, FontWeights.SemiBold));
            left.VerticalAlignment = VerticalAlignment.Center;
            rowGrid.Children.Add(left);
            var right = HStack(8,
                Text(TokenUsage.Compact(report.Usage.Tokens24h), 11, T.OnDark, FontWeights.SemiBold, Theme.MonoFont),
                Text($"{pct:0}%", 10, T.OnDarkFaint, font: Theme.MonoFont));
            right.HorizontalAlignment = HorizontalAlignment.Right;
            rowGrid.Children.Add(right);

            var bar = new Border
            {
                CornerRadius = new CornerRadius(2),
                Height = 3,
                Background = new SolidColorBrush(T.Hairline),
                Child = new Border
                {
                    CornerRadius = new CornerRadius(2),
                    HorizontalAlignment = HorizontalAlignment.Left,
                    Width = Math.Max(2, 280 * report.Usage.Tokens24h / maxTokens),
                    Background = new LinearGradientBrush(T.SydedockCyan, T.SydedockBlue, 0),
                },
            };
            toolList.Children.Add(VStack(4, rowGrid, bar));
        }
        content.Children.Add(Card(VStack(8, HStack(6, Text("按工具用量", 11.5, T.OnDark, FontWeights.SemiBold)), toolList)));

        return BuildPage("Token 用量", "净消耗 · 不含缓存读取", content, back);
    }

    /// 24h/7天/30天 分段控件（Mac 端 analytics 顶部的胶囊分段）
    private static FrameworkElement BuildSegmented(string[] items, int selected, Action<int> onSelect)
    {
        var outer = new Border
        {
            CornerRadius = new CornerRadius(8),
            Background = new SolidColorBrush(T.CardFill),
            BorderBrush = new SolidColorBrush(T.Hairline),
            BorderThickness = new Thickness(0.5),
            Padding = new Thickness(3),
        };
        var grid = new UniformGrid { Rows = 1 };
        for (var i = 0; i < items.Length; i++)
        {
            var idx = i;
            var seg = new Border
            {
                CornerRadius = new CornerRadius(6),
                Margin = new Thickness(2, 0, 2, 0),
                Padding = new Thickness(0, 5, 0, 6),
                Background = i == selected ? new SolidColorBrush(T.Pill) : Brushes.Transparent,
                Child = new TextBlock
                {
                    Text = items[i],
                    FontSize = 11,
                    FontWeight = i == selected ? FontWeights.SemiBold : FontWeights.Normal,
                    Foreground = new SolidColorBrush(i == selected ? T.OnDark : T.OnDarkFaint),
                    TextAlignment = TextAlignment.Center,
                },
                Cursor = Cursors.Hand,
            };
            seg.MouseLeftButtonUp += (_, _) => onSelect(idx);
            grid.Children.Add(seg);
        }
        outer.Child = grid;
        return outer;
    }

    private static FrameworkElement TrendHeader(Dictionary<DateTime, long> hourly)
    {
        var peak = hourly.Count > 0 ? hourly.Values.Max() : 0;
        var right = HStack(5, Controls.IconText("\uE9D2", 8, T.SydedockCyan), Text("峰值", 9, T.OnDarkFaint),
            Text(TokenUsage.Compact(peak), 9.5, T.SydedockCyan, FontWeights.Bold, Theme.MonoFont));
        right.HorizontalAlignment = HorizontalAlignment.Right;
        var g = new Grid();
        g.Children.Add(Text("使用趋势", 11.5, T.OnDark, FontWeights.SemiBold));
        g.Children.Add(right);
        return g;
    }

    /// 折线趋势图（24h 逐时 / 7天 / 30天 逐日，峰值点标记）
    private sealed class TrendChart : FrameworkElement
    {
        private readonly Dictionary<DateTime, long> _hourly;

        public enum Range { Hours24, Days7, Days30 }

        public Range Mode { get; set; } = Range.Hours24;

        public TrendChart(Dictionary<DateTime, long> hourly) => _hourly = hourly;

        protected override void OnRender(DrawingContext dc)
        {
            var t = T;
            var w = ActualWidth;
            var h = ActualHeight;
            if (w <= 0 || h <= 0) return;

            var now = DateTime.Now;
            var buckets = new List<(double x, double v, string label)>();
            long max = 1;

            void AddBucket(DateTime key, string label)
            {
                var v = _hourly.TryGetValue(key, out var x) ? x : 0;
                buckets.Add((0, v, label));
                if (v > max) max = v;
            }

            switch (Mode)
            {
                case Range.Hours24:
                {
                    var start = now.AddHours(-23);
                    for (var i = 0; i < 24; i++)
                    {
                        var hour = start.AddHours(i);
                        AddBucket(new DateTime(hour.Year, hour.Month, hour.Day, hour.Hour, 0, 0),
                            i == 0 ? $"{hour:HH:mm}" : i == 23 ? $"{now:HH:mm}" : "");
                    }
                    break;
                }
                case Range.Days7:
                {
                    var start = now.Date.AddDays(-6);
                    for (var i = 0; i < 7; i++)
                    {
                        var day = start.AddDays(i);
                        var sum = 0L;
                        for (var hr = 0; hr < 24; hr++)
                            sum += _hourly.TryGetValue(day.AddHours(hr), out var x) ? x : 0;
                        buckets.Add((0, sum, $"{day:M/d}"));
                        if (sum > max) max = sum;
                    }
                    break;
                }
                default:
                {
                    var start = now.Date.AddDays(-29);
                    for (var i = 0; i < 30; i++)
                    {
                        var day = start.AddDays(i);
                        var sum = 0L;
                        for (var hr = 0; hr < 24; hr++)
                            sum += _hourly.TryGetValue(day.AddHours(hr), out var x) ? x : 0;
                        buckets.Add((0, sum, i % 5 == 0 ? $"{day:M/d}" : ""));
                        if (sum > max) max = sum;
                    }
                    break;
                }
            }

            // 折线 + 渐变填充
            var pts = new List<Point>();
            for (var i = 0; i < buckets.Count; i++)
            {
                var x = buckets.Count == 1 ? w / 2 : 4 + (double)i / (buckets.Count - 1) * (w - 8);
                var y = h - 14 - (buckets[i].v / (double)max) * (h - 26);
                pts.Add(new Point(x, y));
            }

            // 网格线
            var gridPen = new Pen(new SolidColorBrush(Color.FromArgb(25, t.OnDarkFaint.R, t.OnDarkFaint.G, t.OnDarkFaint.B)), 0.5);
            for (var i = 1; i <= 3; i++)
                dc.DrawLine(gridPen, new Point(4, (h - 14) * i / 4), new Point(w - 4, (h - 14) * i / 4));

            if (pts.Count > 1)
            {
                var fillGeo = new StreamGeometry();
                using (var ctx = fillGeo.Open())
                {
                    ctx.BeginFigure(new Point(pts[0].X, h - 12), true, true);
                    foreach (var p in pts) ctx.LineTo(p, true, true);
                    ctx.LineTo(new Point(pts[^1].X, h - 12), true, true);
                }
                var fillBrush = new LinearGradientBrush(
                    Color.FromArgb(60, t.SydedockCyan.R, t.SydedockCyan.G, t.SydedockCyan.B),
                    Color.FromArgb(5, t.SydedockCyan.R, t.SydedockCyan.G, t.SydedockCyan.B), 90);
                dc.DrawGeometry(fillBrush, null, fillGeo);

                var lineGeo = new StreamGeometry();
                using (var ctx = lineGeo.Open())
                {
                    ctx.BeginFigure(pts[0], false, false);
                    for (var i = 1; i < pts.Count; i++) ctx.LineTo(pts[i], true, false);
                }
                dc.DrawGeometry(null, new Pen(new SolidColorBrush(T.SydedockCyan), 1.4), lineGeo);

                // 峰值点
                var peakIdx = 0;
                for (var i = 1; i < buckets.Count; i++)
                    if (buckets[i].v > buckets[peakIdx].v) peakIdx = i;
                if (buckets[peakIdx].v > 0)
                    dc.DrawEllipse(new SolidColorBrush(T.SydedockCyan), new Pen(Brushes.White, 1), pts[peakIdx], 3, 3);
            }

            // 轴刻度
            for (var i = 0; i < buckets.Count; i++)
            {
                var label = buckets[i].label;
                if (label.Length == 0) continue;
                var px = pts[i].X;
                var fmt = new FormattedText(label, System.Globalization.CultureInfo.InvariantCulture,
                    FlowDirection.LeftToRight, new Typeface(Theme.TextFont, FontStyles.Normal, FontWeights.Normal, FontStretches.Normal),
                    8, new SolidColorBrush(T.OnDarkFaint), VisualTreeHelper.GetDpi(this).PixelsPerDip);
                var lx = Math.Min(Math.Max(px - fmt.Width / 2, 2), w - fmt.Width - 2);
                dc.DrawText(fmt, new Point(lx, h - 12));
            }
        }
    }

    /// 24h 协同节律热力图（24 格）
    private static FrameworkElement BuildHeatmap(Dictionary<DateTime, long> hourly)
    {
        var t = T;
        var header = new Grid();
        header.Children.Add(Text("24h 协同节律", 10.5, T.OnDarkFaint));
        var activeHours = hourly.Count(kv => kv.Key >= DateTime.Now.AddHours(-24) && kv.Value > 0);
        var right = HStack(4, Text("活跃", 9, T.OnDarkFaint), Text($"{activeHours}/24h", 9.5, T.OnDark, FontWeights.SemiBold));
        right.HorizontalAlignment = HorizontalAlignment.Right;
        header.Children.Add(right);

        var strip = new UniformGrid { Rows = 1 };
        var maxV = Math.Max(1, hourly.Where(kv => kv.Key >= DateTime.Now.AddHours(-24)).Select(kv => kv.Value).DefaultIfEmpty(0).Max());
        for (var i = 23; i >= 0; i--)
        {
            var hour = DateTime.Now.AddHours(-i);
            var v = hourly.TryGetValue(new DateTime(hour.Year, hour.Month, hour.Day, hour.Hour, 0, 0), out var x) ? x : 0;
            var intensity = v <= 0 ? 0 : Math.Max(0.15, Math.Sqrt(v / (double)maxV));
            var cell = new Border
            {
                Height = 14,
                Margin = new Thickness(1, 0, 1, 0),
                CornerRadius = new CornerRadius(3),
                Background = new SolidColorBrush(Color.FromArgb(
                    (byte)(30 + intensity * 200), T.SydedockCyan.R, T.SydedockCyan.G, T.SydedockCyan.B)),
            };
            strip.Children.Add(cell);
        }

        return VStack(8, header, strip);
    }

    // MARK: Agent 详情页（双口径总览 + 按模型拆分 + 会话入口）

    public static FrameworkElement BuildAgentDetail(ActivityEngine engine, string agentId, Action back, Action<string, string> openSessions)
    {
        var profile = AgentRegistry.Builtin.FirstOrDefault(p => p.Id == agentId);
        var report = engine.GetTokenReport(agentId);
        var snap = engine.Snapshots.FirstOrDefault(s => s.Id == agentId);

        var content = VStack(8);
        content.Margin = new Thickness(IslandMetrics.PageMargin, 2, IslandMetrics.PageMargin, 8);

        var name = profile?.Name ?? agentId;
        var header = HStack(8, Controls.IconText(profile?.Glyph ?? "\uE7C4", 13, T.OnDarkMuted),
            VStack(1, Text(name, 12.5, T.OnDark, FontWeights.SemiBold),
                Text(snap != null ? $"{snap.Level.Label()} · PID {snap.Pid?.ToString() ?? "—"} · 内存 {snap.MemoryText}" : "离线", 9.5, T.OnDarkFaint)));
        content.Children.Add(header);

        // 双口径总览
        var u = report?.Usage ?? new TokenUsage();
        var totals = HStack(16,
            VStack(2, Text(TokenUsage.Compact(u.Tokens24h), 14, T.OnDark, FontWeights.Bold, Theme.MonoFont), Text("24h 用量", 9, T.OnDarkFaint)),
            VStack(2, Text(u.Cost24h > 0.005 ? $"${u.Cost24h:0.00}" : "—", 14, T.OnDark, FontWeights.Bold, Theme.MonoFont), Text("24h 花费", 9, T.OnDarkFaint)),
            VStack(2, Text(TokenUsage.Compact(u.TokensTotal), 14, T.OnDark, FontWeights.Bold, Theme.MonoFont), Text("累计", 9, T.OnDarkFaint)),
            VStack(2, Text(u.CostTotal > 0.005 ? $"${u.CostTotal:0.00}" : "—", 14, T.OnDark, FontWeights.Bold, Theme.MonoFont), Text("累计花费", 9, T.OnDarkFaint)));
        totals.HorizontalAlignment = HorizontalAlignment.Center;
        content.Children.Add(Card(totals));

        // 按模型拆分
        var models = report?.Models24h ?? [];
        if (models.Count > 0)
        {
            var list = VStack(8);
            var maxTokens = Math.Max(1, models.Max(m => m.Tokens));
            foreach (var m in models)
            {
                var bar = new Border
                {
                    CornerRadius = new CornerRadius(2),
                    Height = 3,
                    Background = new SolidColorBrush(T.Hairline),
                    Child = new Border
                    {
                        CornerRadius = new CornerRadius(2),
                        HorizontalAlignment = HorizontalAlignment.Left,
                        Width = Math.Max(2, 250 * m.Tokens / maxTokens),
                        Background = new LinearGradientBrush(T.SydedockCyan, T.SydedockBlue, 0),
                    },
                };
                var rowGrid = new Grid();
                rowGrid.Children.Add(Text(m.Model, 10.5, T.OnDark));
                var right = HStack(8,
                    Text(TokenUsage.Compact(m.Tokens), 10.5, T.OnDark, FontWeights.SemiBold, Theme.MonoFont),
                    Text(m.Cost > 0.005 ? $"${m.Cost:0.00}" : "", 9.5, T.SydedockAmber, font: Theme.MonoFont));
                right.HorizontalAlignment = HorizontalAlignment.Right;
                rowGrid.Children.Add(right);

                var row = VStack(4, rowGrid, bar);
                var model = m.Model;
                row.Cursor = Cursors.Hand;
                row.MouseLeftButtonUp += (_, _) => openSessions(agentId, model);
                list.Children.Add(row);
            }
            content.Children.Add(Card(VStack(8, Text("按模型拆分（24h）", 11, T.OnDark, FontWeights.SemiBold), list)));
        }
        else
        {
            content.Children.Add(Card(VStack(4,
                Text("未发现本地明细", 11, T.OnDarkFaint, FontWeights.SemiBold),
                Text("该档案登记的明细源本轮没有可读取的用量记录；「没取到」不等于「真的零用量」。", 9.5, T.OnDarkFaint))));
        }

        return BuildPage(name, $"{profile?.Emoji ?? "🤖"} Agent 详情 · 双口径总览 + 按模型拆分", content, back);
    }

    // MARK: 会话列表页（该模型最近会话文件）

    public static FrameworkElement BuildSessions(ActivityEngine engine, string agentId, string modelId, Action back)
    {
        var profile = AgentRegistry.Builtin.FirstOrDefault(p => p.Id == agentId);
        var content = VStack(6);
        content.Margin = new Thickness(IslandMetrics.PageMargin, 2, IslandMetrics.PageMargin, 8);

        var files = new List<(string path, DateTime mtime, long size)>();
        if (profile != null)
        {
            foreach (var root in profile.TokenRoots)
            {
                if (!Directory.Exists(root)) continue;
                try
                {
                    foreach (var f in Directory.GetFiles(root, "*.jsonl", SearchOption.AllDirectories))
                    {
                        var fi = new FileInfo(f);
                        files.Add((f, fi.LastWriteTime, fi.Length));
                    }
                }
                catch { }
            }
        }
        var recent = files.OrderByDescending(f => f.mtime).Take(12).ToList();

        if (recent.Count == 0)
        {
            content.Children.Add(Card(Text("没有找到会话明细文件", 10.5, T.OnDarkFaint)));
        }
        else
        {
            foreach (var (path, mtime, size) in recent)
            {
                var rowGrid = new Grid();
                var name = System.IO.Path.GetFileName(path);
                rowGrid.Children.Add(VStack(2, Text(name, 10.5, T.OnDark), Text($"{TimeAgoText.Format(DateTimeOffset.Now - mtime)} · {TokenUsage.Compact((long)(size * 0.08))} tokens", 9, T.OnDarkFaint)));
                var right = Text("\uE76C", 9, T.OnDarkFaint);
                right.HorizontalAlignment = HorizontalAlignment.Right;
                rowGrid.Children.Add(right);
                var row = Card(rowGrid, padding: 8);
                row.Cursor = Cursors.Hand;
                row.ToolTip = "跳转目录：" + System.IO.Path.GetDirectoryName(path);
                row.MouseLeftButtonUp += (_, _) =>
                {
                    try
                    {
                        var dir = System.IO.Path.GetDirectoryName(path) ?? path;
                        System.Diagnostics.Process.Start("explorer.exe", $"/select,\"{path}\"");
                    }
                    catch { }
                };
                content.Children.Add(row);
            }
        }

        return BuildPage(modelId.Length > 0 ? modelId : "会话", $"{profile?.Name ?? agentId} · 最近会话（时间 / 大小）", content, back);
    }

    // MARK: 实时流水抽屉（最近事件 + 各 Agent 快照）

    public static FrameworkElement BuildLiveStream(ActivityEngine engine, string? agentId, Action back)
    {
        var content = VStack(6);
        content.Margin = new Thickness(IslandMetrics.PageMargin, 2, IslandMetrics.PageMargin, 8);

        var snap = agentId == null ? null : engine.Snapshots.FirstOrDefault(s => s.Id == agentId);
        var title = snap?.Profile.Name ?? "实时流水";
        if (snap != null)
        {
            content.Children.Add(Card(VStack(4,
                HStack(8, Text("当前状态", 10, T.OnDarkFaint), Controls.StatusPill(snap.Level)),
                Text(snap.CurrentAction != null ? $"正在执行: {snap.CurrentAction}" : "暂无在途动作", 10, T.OnDark),
                Text($"CPU {(snap.CpuPercent?.ToString("0.0") ?? "—")}% · 内存 {snap.MemoryText} · 最后活动 {snap.LastActivityText}", 9.5, T.OnDarkFaint))));
        }

        var ev = engine.LatestEvent;
        content.Children.Add(Card(VStack(4,
            Text("最近事件", 10, T.OnDarkFaint, FontWeights.SemiBold),
            Text(ev != null ? $"[{ev.Timestamp:HH:mm:ss}] {ev.SummaryText}" : "暂无事件", 10, ev != null ? T.OnDark : T.OnDarkFaint),
            Text(ev?.Detail ?? "", 9.5, T.OnDarkFaint))));

        content.Children.Add(Card(VStack(4,
            Text("监控可信度", 10, T.OnDarkFaint, FontWeights.SemiBold),
            Text(snap?.SessionProbeHealth != null
                ? snap.SessionProbeHealth.DiagnosticText
                : "会话源可读；判定与 doctor 共用同一份实现。", 9.5, T.OnDarkFaint))));

        return BuildPage(title, "实时事件与状态抽屉", content, back);
    }
}
