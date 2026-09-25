using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Threading;
using AgentIsland.Core;

namespace AgentIsland.UI;

// MARK: - 通用小组件工厂（对应 Mac 端 IslandComponents / AgentRingView / StatusIcons）

public static class Controls
{
    public static Theme T => ThemeManager.Current;

    // ---- 基础工厂 ----

    public static TextBlock Text(string s, double size, Color color, FontWeight? weight = null,
        FontFamily? font = null, TextTrimming? trimming = null, VerticalAlignment vertical = VerticalAlignment.Center)
    {
        var tb = new TextBlock
        {
            Text = s,
            FontFamily = font ?? Theme.TextFont,
            FontSize = size,
            Foreground = new SolidColorBrush(color),
            VerticalAlignment = vertical,
        };
        if (weight != null) tb.FontWeight = weight.Value;
        if (trimming != null) tb.TextTrimming = trimming.Value;
        return tb;
    }

    public static TextBlock IconText(string glyph, double size, Color color)
    {
        var tb = new TextBlock
        {
            Text = glyph,
            FontFamily = Theme.IconFont,
            FontSize = size,
            Foreground = new SolidColorBrush(color),
        };
        return tb;
    }

    public static Border Card(double corner, Color fill, Color border, double borderThickness = 0.5)
    {
        var b = new Border
        {
            CornerRadius = new CornerRadius(corner),
            Background = new SolidColorBrush(fill),
            BorderBrush = new SolidColorBrush(border),
            BorderThickness = new Thickness(borderThickness),
        };
        return b;
    }

    public static DockPanel Divider()
    {
        var rect = new System.Windows.Shapes.Rectangle
        {
            Height = IslandMetrics.DividerHeight,
            Fill = new SolidColorBrush(T.Hairline),
            Opacity = 0.9,
        };
        var dp = new DockPanel { LastChildFill = false };
        DockPanel.SetDock(rect, Dock.Top);
        dp.Children.Add(rect);
        return dp;
    }

    public static System.Windows.Shapes.Rectangle DividerRect() => new()
    {
        Height = IslandMetrics.DividerHeight,
        Fill = new SolidColorBrush(T.Hairline),
        Opacity = 0.9,
    };

    /// 顶栏圆形图标按钮（TopBarChip：chipFill 底、hover 升级）
    public static Border IconButton(string glyph, string tooltip, Action onClick, double size = 10, double box = 22)
    {
        var icon = IconText(glyph, size, T.OnDarkFaint);
        var border = new Border
        {
            Width = box,
            Height = box,
            CornerRadius = new CornerRadius(6),
            Background = new SolidColorBrush(T.ChipFill),
            BorderBrush = new SolidColorBrush(Color.FromArgb(20, 255, 255, 255)),
            BorderThickness = new Thickness(0.5),
            Child = new Grid { Children = { icon } },
            Cursor = Cursors.Hand,
            ToolTip = tooltip,
        };
        border.MouseEnter += (_, _) =>
        {
            icon.Foreground = new SolidColorBrush(T.OnDarkMuted);
            border.Background = new SolidColorBrush(T.HoverFill);
        };
        border.MouseLeave += (_, _) =>
        {
            icon.Foreground = new SolidColorBrush(T.OnDarkFaint);
            border.Background = new SolidColorBrush(T.ChipFill);
        };
        border.MouseLeftButtonUp += (_, _) => onClick();
        return border;
    }

    /// 状态药丸（行右侧：待机/工作中…）
    public static Border StatusPill(ActivityLevel level)
    {
        var pill = new Border
        {
            CornerRadius = new CornerRadius(999),
            Padding = new Thickness(6, 2, 6, 2.5),
            Background = new SolidColorBrush(T.LevelBackground(level)),
            BorderBrush = new SolidColorBrush(T.LevelBorder(level)),
            BorderThickness = new Thickness(0.5),
            Child = Text(level.Label(), 9.5, T.LevelForeground(level), FontWeights.SemiBold),
        };
        return pill;
    }

    /// Token 徽标胶囊（"277M" / "1.2G"）
    public static Border TokenBadgePill(string text, bool emphasis = false)
    {
        var pill = new Border
        {
            CornerRadius = new CornerRadius(4),
            Padding = new Thickness(5, 1.5, 5, 2),
            Background = new SolidColorBrush(T.Pill),
            BorderBrush = new SolidColorBrush(T.Hairline),
            BorderThickness = new Thickness(0.5),
            Child = Text(text, 9.5, emphasis ? T.OnDark : T.OnDarkMuted, FontWeights.SemiBold, Theme.MonoFont),
            VerticalAlignment = VerticalAlignment.Center,
        };
        return pill;
    }
}

// MARK: - 环形微仪表盘（与 macOS AgentRingView 同规则：
// 外圈水位分级环 + 居中 Glyph + 工作态旋转微弧 + 等待态呼吸环）

public sealed class AgentRing : FrameworkElement
{
    private AgentSnapshot? _snapshot;
    private double _angle;
    private double _pulse;
    private readonly DispatcherTimer _timer;

    public AgentRing()
    {
        _timer = new DispatcherTimer(DispatcherPriority.Render)
        {
            Interval = TimeSpan.FromMilliseconds(33), // ~30fps，微控件足够顺滑
        };
        _timer.Tick += (_, _) =>
        {
            _angle = (_angle + 4) % 360;
            _pulse = Math.Sin(DateTimeOffset.Now.ToUnixTimeMilliseconds() / 300.0) * 0.5 + 0.5;
            InvalidateVisual();
        };
        IsVisibleChanged += (_, e) =>
        {
            if ((bool)e.NewValue) StartIfAnimated();
            else _timer.Stop();
        };
    }

    public void Update(AgentSnapshot? snapshot)
    {
        _snapshot = snapshot;
        StartIfAnimated();
        InvalidateVisual();
    }

    private void StartIfAnimated()
    {
        var animated = _snapshot is { Level: ActivityLevel.Working or ActivityLevel.Attention };
        if (animated && !_timer.IsEnabled) _timer.Start();
        else if (!animated && _timer.IsEnabled) _timer.Stop();
    }

    private static double Progress(AgentSnapshot s) => s.Level switch
    {
        ActivityLevel.Working => Math.Max(0.35, Math.Min(0.35 + (s.CpuPercent ?? 0) / 100.0 * 0.65, 1.0)),
        ActivityLevel.Attention => 0.78,
        ActivityLevel.Completed => 0.45,
        ActivityLevel.Idle when s.TokenUsage is { } u && u.Tokens24h > 0 =>
            Math.Max(0.15, Math.Min(u.Tokens24h / 200_000.0, 1.0)),
        _ => 0,
    };

    private Color RingColor(AgentSnapshot s, Theme t) => s.Level switch
    {
        ActivityLevel.Working when s.IsHung == true => t.RingRed,
        ActivityLevel.Working when (s.CpuPercent ?? 0) >= 80 => t.RingOrange,
        ActivityLevel.Working => t.RingGreen,
        ActivityLevel.Attention => t.RingYellow,
        ActivityLevel.Completed => t.RingGreen,
        ActivityLevel.Idle when s.TokenUsage is { } u && u.Tokens24h >= 200_000 => t.RingOrange,
        ActivityLevel.Idle when s.TokenUsage is { } u2 && u2.Tokens24h >= 50_000 => t.RingYellow,
        ActivityLevel.Idle when s.TokenUsage is { } u3 && u3.Tokens24h > 0 => t.RingGreen,
        _ => t.RingTrack,
    };

    protected override void OnRender(DrawingContext dc)
    {
        var t = ThemeManager.Current;
        var size = Math.Min(ActualWidth, ActualHeight);
        if (size <= 0 || _snapshot == null) return;
        var s = _snapshot;
        var corner = size * 0.28;
        var stroke = size >= 34 ? 2.5 : 2.0;
        var glyphSize = size >= 34 ? 12.5 : 10.5;
        var inset = stroke / 2;
        var bg = t.IsDark ? Color.FromArgb(10, 255, 255, 255) : Color.FromArgb(230, 255, 255, 255);
        var trackPen = new Pen(new SolidColorBrush(t.RingTrack), stroke);
        var center = new Point(ActualWidth / 2, ActualHeight / 2);
        var rect = new Rect(center.X - size / 2 + inset, center.Y - size / 2 + inset,
            size - stroke, size - stroke);
        var radiusRect = new Rect(center.X - size / 2, center.Y - size / 2, size, size);

        // 1. 底轨（圆角超椭圆跑道）
        dc.DrawRoundedRectangle(new SolidColorBrush(bg), trackPen, radiusRect, corner, corner);

        // 2. 外圈分级彩色进度环
        var progress = Progress(s);
        if (progress > 0)
        {
            var color = RingColor(s, t);
            var pen = new Pen(new SolidColorBrush(color), stroke) { StartLineCap = PenLineCap.Round, EndLineCap = PenLineCap.Round };
            // 用几何近似圆角矩形环：走矩形路径的参数化描边
            var geo = RoundedRectArc(rect, corner, progress);
            dc.DrawGeometry(null, pen, geo);
        }

        // 3. 内圈活动动效
        if (s.Level == ActivityLevel.Working)
        {
            var color = RingColor(s, t);
            var inner = stroke + 3;
            var innerRect = new Rect(center.X - size / 2 + inner, center.Y - size / 2 + inner,
                size - 2 * inner, size - 2 * inner);
            var arcGeo = RoundedRectArc(innerRect, corner, 0.35, startAngle: _angle - 90);
            var gradient = new LinearGradientBrush(
                Color.FromArgb(0, color.R, color.G, color.B),
                Color.FromArgb(240, color.R, color.G, color.B), 0);
            var pen = new Pen(gradient, Math.Max(1.4, stroke * 0.65)) { StartLineCap = PenLineCap.Round, EndLineCap = PenLineCap.Round };
            dc.DrawGeometry(null, pen, arcGeo);
        }
        else if (s.Level == ActivityLevel.Attention)
        {
            var color = t.RingYellow;
            var inner = stroke + 3;
            var innerRect = new Rect(center.X - size / 2 + inner, center.Y - size / 2 + inner,
                size - 2 * inner, size - 2 * inner);
            var innerGeo = RoundedRectArc(innerRect, corner, 1.0);
            var alphaOuter = (byte)(0.55 + 0.35 * _pulse);
            var alphaInner = (byte)(0.18 + 0.6 * _pulse);
            var outerPen = new Pen(new SolidColorBrush(Color.FromArgb(alphaInner, color.R, color.G, color.B)),
                Math.Max(1.4, stroke * 0.7));
            var innerPen = new Pen(new SolidColorBrush(Color.FromArgb(alphaOuter, color.R, color.G, color.B)),
                Math.Max(1.2, stroke * 0.5));
            dc.DrawGeometry(null, outerPen, innerGeo);
            dc.DrawGeometry(null, innerPen, innerGeo);
        }

        // 4. 居中 Glyph
        var opacity = s.Level == ActivityLevel.Offline ? 0.35 : (s.Level == ActivityLevel.Idle ? 0.65 : 1.0);
        var glyph = FormattedGlyph(s.Profile.Glyph, glyphSize, t.OnDark, opacity);
        var glyphPos = new Point(center.X - glyph.Width / 2, center.Y - glyph.Height / 2);
        dc.DrawText(glyph, glyphPos);
    }

    private FormattedText FormattedGlyph(string glyph, double size, Color color, double opacity)
    {
        return new FormattedText(glyph, System.Globalization.CultureInfo.InvariantCulture,
            FlowDirection.LeftToRight, new Typeface(Theme.IconFont, FontStyles.Normal, FontWeights.Normal, FontStretches.Normal), size,
            new SolidColorBrush(color) { Opacity = opacity },
            VisualTreeHelper.GetDpi(this).PixelsPerDip);
    }

    /// 圆角矩形路径上按比例 progress 取弧（自顶部顺时针）。
    /// 简化实现：把圆角矩形环视作 4 段直边 + 4 个 90° 圆弧，按弧长参数化。
    private static Geometry RoundedRectArc(Rect rect, double corner, double progress, double startAngle = -90)
    {
        corner = Math.Min(corner, Math.Min(rect.Width, rect.Height) / 2);
        var geo = new StreamGeometry();
        // 参数化：沿矩形周长的位置 → 点。周长从顶部中点顺时针。
        double Perimeter() => 2 * (rect.Width + rect.Height) - 8 * corner + 2 * Math.PI * corner;
        double ArcCorner() => Math.PI * corner / 2;

        Point PointAt(double d)
        {
            double straightW = rect.Width - 2 * corner, straightH = rect.Height - 2 * corner;
            double seg1 = ArcCorner() / 2, seg2 = straightW, seg3 = ArcCorner(), seg4 = straightH,
                   seg5 = ArcCorner(), seg6 = straightW, seg7 = ArcCorner(), seg8 = straightH, seg9 = ArcCorner() / 2;
            // 顶部中点 → 右上角弧 → 右边 → 右下角弧 → 底边 → 左下角弧 → 左边 → 左上角弧 → 回顶部中点
            double acc = 0;
            double arcR = corner;

            double InSeg(double len)
            {
                var local = d - acc; acc += len; return Math.Clamp(local, 0, len);
            }
            var topMidX = rect.Left + rect.Width / 2;
            if (d < seg1) // 右上角弧前半
            {
                var a = InSeg(seg1) / seg1 * 45;
                return ArcPoint(topMidX + arcR * Math.Sin(Deg(-90 + a)), rect.Top + corner - arcR * Math.Cos(Deg(-90 + a)), 0);
            }
            acc = seg1;
            if (d < seg1 + seg2)
            {
                var x = InSeg(seg2);
                return new Point(rect.Left + corner + x, rect.Top);
            }
            acc = seg1 + seg2;
            if (d < seg1 + seg2 + seg3)
            {
                var a = InSeg(seg3) / seg3 * 90;
                return ArcPoint(rect.Right - corner + arcR * Math.Sin(Deg(a)), rect.Top + corner - arcR * Math.Cos(Deg(a)), 0);
            }
            acc = seg1 + seg2 + seg3;
            if (d < seg1 + seg2 + seg3 + seg4)
            {
                var y = InSeg(seg4);
                return new Point(rect.Right, rect.Top + corner + y);
            }
            acc = seg1 + seg2 + seg3 + seg4;
            if (d < seg1 + seg2 + seg3 + seg4 + seg5)
            {
                var a = InSeg(seg5) / seg5 * 90;
                return ArcPoint(rect.Right - corner + arcR * Math.Cos(Deg(a)), rect.Bottom - corner + arcR * Math.Sin(Deg(a)), 0);
            }
            acc = seg1 + seg2 + seg3 + seg4 + seg5;
            if (d < seg1 + seg2 + seg3 + seg4 + seg5 + seg6)
            {
                var x = InSeg(seg6);
                return new Point(rect.Right - corner - x, rect.Bottom);
            }
            acc = seg1 + seg2 + seg3 + seg4 + seg5 + seg6;
            if (d < seg1 + seg2 + seg3 + seg4 + seg5 + seg6 + seg7)
            {
                var a = InSeg(seg7) / seg7 * 90;
                return ArcPoint(rect.Left + corner - arcR * Math.Sin(Deg(a)), rect.Bottom - corner + arcR * Math.Cos(Deg(a)), 0);
            }
            acc = seg1 + seg2 + seg3 + seg4 + seg5 + seg6 + seg7;
            if (d < seg1 + seg2 + seg3 + seg4 + seg5 + seg6 + seg7 + seg8)
            {
                var y = InSeg(seg8);
                return new Point(rect.Left, rect.Bottom - corner - y);
            }
            acc = seg1 + seg2 + seg3 + seg4 + seg5 + seg6 + seg7 + seg8;
            {
                var a = InSeg(seg9) / seg9 * 45;
                return ArcPoint(rect.Left + corner - arcR * Math.Cos(Deg(a)), rect.Top + corner - arcR * Math.Sin(Deg(a)), 0);
            }

            static Point ArcPoint(double x, double y, double _) => new(x, y);
        }

        static double Deg(double d) => d * Math.PI / 180.0;

        using (var ctx = geo.Open())
        {
            var perimeter = Perimeter();
            var length = perimeter * Math.Clamp(progress, 0, 1);
            var start = (startAngle + 90) / 360.0 * perimeter; // startAngle=-90 → 顶部中点
            const double step = 0.012;
            var first = true;
            for (var d = 0.0; d <= length; d += step * perimeter)
            {
                var p = PointAt((start + d) % perimeter);
                if (first) { ctx.BeginFigure(p, false, false); first = false; }
                else ctx.LineTo(p, true, false);
            }
        }
        geo.Freeze();
        return geo;
    }
}

// MARK: - 活动点阵（ActivityMatrixDots：5 点活跃度微矩阵）

public sealed class ActivityDots : FrameworkElement
{
    private readonly bool[] _lit = new bool[5];

    public void Update(AgentSnapshot? snapshot)
    {
        // 点阵语义：最近活动越新、点亮越多（10s/1m/5m/15m/60m 窗口）
        var ago = snapshot?.LastActivityAgo;
        _lit[0] = ago is < 10;
        _lit[1] = ago is < 60;
        _lit[2] = ago is < 300;
        _lit[3] = ago is < 900;
        _lit[4] = ago is < 3600;
        if (snapshot is { Level: ActivityLevel.Working }) _lit[0] = _lit[1] = true;
        InvalidateVisual();
    }

    protected override void OnRender(DrawingContext dc)
    {
        var t = ThemeManager.Current;
        const double dot = 3.5, gap = 2.2;
        var y = ActualHeight / 2;
        var totalW = 5 * dot + 4 * gap;
        var x = (ActualWidth - totalW) / 2;
        for (var i = 0; i < 5; i++)
        {
            var color = _lit[i] ? t.RingGreen : Color.FromArgb(60, t.OnDarkFaint.R, t.OnDarkFaint.G, t.OnDarkFaint.B);
            var brush = new SolidColorBrush(color);
            if (i == 4 && _lit[4]) brush = new SolidColorBrush(Color.FromArgb(220, t.RingGreen.R, t.RingGreen.G, t.RingGreen.B));
            dc.DrawEllipse(brush, null, new Point(x + dot / 2, y), dot / 2, dot / 2);
            x += dot + gap;
        }
    }
}
