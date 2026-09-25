using System.Windows;
using System.Windows.Media;
using AgentIsland.Core;

namespace AgentIsland.UI;

// MARK: - 设计令牌（与 macOS 端 Theme.swift 同源的色板）
// 深色 = 黑曜石荧光；浅色 = 白瓷工控（对比度按 WCAG AA 校准）。

public sealed class ThemeColors
{
    // 基色阶（Tailwind slate/emerald/amber…）
    public static readonly Color Slate50 = Col(0xF8FAFC);
    public static readonly Color Slate100 = Col(0xF1F5F9);
    public static readonly Color Slate200 = Col(0xE2E8F0);
    public static readonly Color Slate300 = Col(0xCBD5E1);
    public static readonly Color Slate400 = Col(0x94A3B8);
    public static readonly Color Slate500 = Col(0x64748B);
    public static readonly Color Slate600 = Col(0x475569);
    public static readonly Color Slate700 = Col(0x334155);
    public static readonly Color Emerald50 = Col(0xECFDF5);
    public static readonly Color Emerald200 = Col(0xA7F3D0);
    public static readonly Color Emerald700 = Col(0x047857);
    public static readonly Color Amber50 = Col(0xFFFBE8 + 0x02); // 0xfffbeb
    public static readonly Color Amber200 = Col(0xFDE68A);
    public static readonly Color Amber700 = Col(0xB45309);
    public static readonly Color Amber900 = Col(0x78350F);
    public static readonly Color Sky800 = Col(0x075985);
    public static readonly Color InkGreen = Col(0x157F3C);
    public static readonly Color InkAmber = Col(0x8F6A00);

    private static Color Col(uint hex) => Color.FromRgb((byte)(hex >> 16), (byte)((hex >> 8) & 0xFF), (byte)(hex & 0xFF));
}

public sealed class Theme
{
    public bool IsDark { get; }

    public Theme(bool isDark) => IsDark = isDark;

    private static Color Rgba(uint hex, double alpha) => Color.FromArgb(
        (byte)Math.Round(alpha * 255),
        (byte)((hex >> 16) & 0xFF), (byte)((hex >> 8) & 0xFF), (byte)(hex & 0xFF));

    // ---- 灵动岛主体 ----
    public Color ObsidianBase => IsDark ? Rgba(0x0C0D14, 1) : Rgba(0xF8FAFC, 1);
    public Color ObsidianCard => IsDark ? Rgba(0x161722, 1) : Colors.White;
    public Color CardFill => IsDark ? Rgba(0xFFFFFF, 0.07) : Rgba(0xFFFFFF, 0.94);
    public Color CardHoverFill => IsDark ? Rgba(0xFFFFFF, 0.13) : Rgba(0xFFFFFF, 1.0);
    public Color CardBorder => IsDark ? Rgba(0xFFFFFF, 0.14) : Rgba(0xE2E8F0, 0.85);
    public Color CardBorderHover => IsDark ? Rgba(0xFFFFFF, 0.28) : Rgba(0xCBD5E1, 0.95);
    public Color Hairline => IsDark ? Rgba(0xFFFFFF, 0.14) : Rgba(0xE2E8F0, 0.90);
    public Color Pill => IsDark ? Rgba(0xFFFFFF, 0.09) : Rgba(0xF1F5F9, 0.95);
    public Color ChipFill => IsDark ? Rgba(0xFFFFFF, 0.08) : Rgba(0xF1F5F9, 0.90);
    public Color HoverFill => IsDark ? Rgba(0xFFFFFF, 0.12) : Rgba(0xFFFFFF, 0.96);

    /// 玻璃卡底（无系统级毛玻璃，用近不透明冷炭/白瓷近似 Mac 的 ultraThinMaterial+光罩）
    public Color GlassTop => IsDark ? Rgba(0x0D0E14, 0.94) : Rgba(0xF6F8FB, 0.97);
    public Color GlassBottom => IsDark ? Rgba(0x05060A, 0.97) : Rgba(0xEBEEF3, 0.985);
    public Color GlassSpecularTop => IsDark ? Rgba(0xFFFFFF, 0.40) : Rgba(0xFFFFFF, 0.96);
    public Color GlassSpecularBottom => IsDark ? Rgba(0xFFFFFF, 0.10) : Rgba(0x000000, 0.08);

    // ---- 文本 ----
    public Color OnDark => IsDark ? Rgba(0xF8FAFC, 1) : Rgba(0x0F172A, 1);
    public Color OnDarkMuted => IsDark ? Rgba(0xCFD4DC, 1) : Rgba(0x334155, 1);
    public Color OnDarkFaint => IsDark ? Rgba(0x94A3B8, 1) : Rgba(0x475569, 1);

    // ---- 荧光强调 ----
    public Color SydedockCyan => IsDark ? Rgba(0x00E1FF, 1) : Rgba(0x075985, 1);
    public Color SydedockBlue => IsDark ? Rgba(0x38BDF8, 1) : Rgba(0x1D4ED8, 1);
    public Color SydedockAmber => IsDark ? Rgba(0xFFD60A, 1) : Rgba(0x78350F, 1);
    public Color SydedockEmerald => IsDark ? Rgba(0x30D158, 1) : Rgba(0x15803D, 1);

    // ---- 五态 ----
    public Color StatusWorking => IsDark ? Rgba(0x30D158, 1) : Rgba(0x157F3C, 1);
    public Color StatusIdle => IsDark ? Rgba(0xFFD60A, 1) : Rgba(0x8F6A00, 1);
    public Color StatusOffline => IsDark ? Rgba(0x8E8E93, 1) : Rgba(0x5C5C61, 1);
    public Color WarningOrange => IsDark ? Rgba(0xFF9500, 1) : Rgba(0xB45309, 1);
    public Color DangerRed => IsDark ? Rgba(0xFF3B30, 1) : Rgba(0xD32F2F, 1);
    public Color ActionBlue => Rgba(0x0066CC, 1);

    // ---- docked 细条 ----
    public Color SliverFill(bool working, bool alert) =>
        alert ? (IsDark ? Rgba(0x3D0A0A, 0.85) : Rgba(0xD32F2F, 0.85))
              : (IsDark ? Rgba(0x08080C, working ? 0.82 : 0.70) : Rgba(0xFFFFFF, working ? 0.95 : 0.90));
    public Color SliverStroke(bool working, bool alert) =>
        alert ? DangerRed
        : working ? SydedockEmerald
        : (IsDark ? Rgba(0xFFFFFF, 0.25) : Rgba(0xCBD5E1, 0.80));
    public Color GlowAlert => Rgba(0xFF3B30, 1);
    public Color GlowWorking => Rgba(0x10B981, 1);
    public Color GlowIdle => Rgba(0xFFD60A, 1);

    // ---- 环形水位色板（Palette） ----
    public Color RingTrack => IsDark ? Rgba(0x22222A, 1) : Rgba(0xE2E8F0, 1);
    public Color RingGreen => IsDark ? Rgba(0x28E07B, 1) : Rgba(0x157F3C, 1);
    public Color RingYellow => IsDark ? Rgba(0xE3C567, 1) : Rgba(0x8F6A00, 1);
    public Color RingOrange => IsDark ? Rgba(0xFF6B35, 1) : Rgba(0xC23A00, 1);
    public Color RingRed => IsDark ? Rgba(0xFF3B30, 1) : Rgba(0xC62828, 1);

    // ---- 设置窗口画布 ----
    public Color Canvas => IsDark ? Rgba(0x151517, 1) : Rgba(0xF5F5F7, 1);
    public Color Parchment => IsDark ? Rgba(0x1E1E20, 1) : Colors.White;

    // ---- 活动等级色阶（岛内唯一「状态 → 颜色」来源） ----
    public Color LevelColor(ActivityLevel level) => level switch
    {
        ActivityLevel.Working or ActivityLevel.Completed => StatusWorking,
        ActivityLevel.Attention => WarningOrange,
        ActivityLevel.Idle => StatusIdle,
        _ => StatusOffline,
    };

    public Color LevelForeground(ActivityLevel level) => !IsDark
        ? level switch
        {
            ActivityLevel.Working or ActivityLevel.Completed => ThemeColors.Emerald700,
            ActivityLevel.Attention => ThemeColors.Amber700,
            ActivityLevel.Idle => ThemeColors.Slate600,
            _ => ThemeColors.Slate500,
        }
        : LevelColor(level);

    public Color LevelBackground(ActivityLevel level) => !IsDark
        ? level switch
        {
            ActivityLevel.Working or ActivityLevel.Completed => ThemeColors.Emerald50,
            ActivityLevel.Attention => ThemeColors.Amber50,
            ActivityLevel.Idle => ThemeColors.Slate100,
            _ => ThemeColors.Slate50,
        }
        : Color.FromArgb(31, LevelColor(level).R, LevelColor(level).G, LevelColor(level).B); // 12%

    public Color LevelBorder(ActivityLevel level) => !IsDark
        ? level switch
        {
            ActivityLevel.Working or ActivityLevel.Completed => ThemeColors.Emerald200,
            ActivityLevel.Attention => ThemeColors.Amber200,
            ActivityLevel.Idle or ActivityLevel.Offline => ThemeColors.Slate200,
            _ => ThemeColors.Slate200,
        }
        : Color.FromArgb(82, LevelColor(level).R, LevelColor(level).G, LevelColor(level).B); // 32%

    // ---- 字体 ----
    public static FontFamily TextFont { get; } = new("Segoe UI, Microsoft YaHei UI");
    public static FontFamily MonoFont { get; } = new("Cascadia Mono, Consolas, Microsoft YaHei UI");
    public static FontFamily IconFont { get; } = new("Segoe Fluent Icons, Segoe MDL2 Assets");
}

// MARK: - 几何度量（与 macOS 端 IslandMetrics.swift 同源）

public static class IslandMetrics
{
    /// 反向倒角占据的边缘区域，内容必须避让
    public const double NotchInset = 10;

    public const double CardWidth = 330;
    public const double ExpandedMaxHeight = 460;

    public const double TopSliverWidth = 140;
    public const double TopSliverHeight = 6;
    public const double RightSliverWidth = 6;
    public const double RightSliverHeight = 120;

    public const double HeaderPaddingTop = 12;
    public const double HeaderPaddingBottom = 8;
    public const double HeaderContentHeight = 34;
    public static double HeaderHeight => HeaderPaddingTop + HeaderContentHeight + HeaderPaddingBottom;

    public const double DividerHeight = 1;
    public const double RowHeight = 52;
    public const double ListVerticalPadding = 6;
    public const double ListExtraHeight = 10;
    public const double ListMaxHeight = 340;
    public const double EmptyStatePaddingVertical = 22;
    public const double EmptyStateHeight = 121;
    public const double SummaryBarHeight = 28;
    public const double DetailHeaderHeight = 47;
    public const double DetailContentHeight = 310;
    public const double RingsShelfHeight = 40;
    public const double EventBannerCollapsedHeight = 66;
    public const double EventBannerExpandedHeight = 142;

    public const double RadiusSm = 8;
    public const double RadiusMd = 11;
    public const double RadiusLg = 18;
    public const double PageMargin = 14;

    public static double ChromeHeight(bool hasSummary, bool hasRings, bool hasEvent, bool eventExpanded)
    {
        var summary = hasSummary ? SummaryBarHeight + DividerHeight : 0;
        var rings = hasRings ? RingsShelfHeight + DividerHeight : 0;
        var bannerH = eventExpanded ? EventBannerExpandedHeight : EventBannerCollapsedHeight;
        var eventH = hasEvent ? bannerH + DividerHeight : 0;
        return 2 * NotchInset + HeaderHeight + DividerHeight + rings + eventH + summary;
    }

    public static double ListHeight(int visibleCount, bool hasSummary, bool hasRings, bool hasEvent, bool eventExpanded)
    {
        var chrome = ChromeHeight(hasSummary, hasRings, hasEvent, eventExpanded);
        var contentHeight = Math.Max(visibleCount, 1) * RowHeight + ListExtraHeight;
        var available = Math.Max(ExpandedMaxHeight - chrome, RowHeight);
        return Math.Min(Math.Min(contentHeight, ListMaxHeight), available);
    }

    public static double ExpandedHeight(string route, int visibleCount, bool hasSummary, bool hasRings = false, bool hasEvent = false, bool eventExpanded = false)
    {
        if (route == "list")
        {
            var chrome = ChromeHeight(hasSummary, hasRings, hasEvent, eventExpanded);
            if (visibleCount == 0) return Math.Min(chrome + EmptyStateHeight, ExpandedMaxHeight);
            var list = ListHeight(visibleCount, hasSummary, hasRings, hasEvent, eventExpanded);
            return Math.Min(chrome + list, ExpandedMaxHeight);
        }
        return Math.Min(2 * NotchInset + DetailHeaderHeight + DividerHeight + DetailContentHeight, ExpandedMaxHeight);
    }
}

// MARK: - 主题管理器（热切换）

public static class ThemeManager
{
    public static Theme Current { get; private set; } = new(true);
    public static event Action? ThemeChanged;

    public static void Apply(IslandAppearance mode)
    {
        var isDark = mode switch
        {
            IslandAppearance.Dark => true,
            IslandAppearance.Light => false,
            _ => IsSystemDark(),
        };
        Current = new Theme(isDark);
        ThemeChanged?.Invoke();
    }

    public static bool IsSystemDark()
    {
        try
        {
            using var key = Microsoft.Win32.Registry.CurrentUser.OpenSubKey(
                @"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
            return key?.GetValue("AppsUseLightTheme") is int v && v == 0;
        }
        catch
        {
            return true;
        }
    }
}

// MARK: - 反向倒角一体化贴边形状（与 macOS 端 SideNotchShape 同源）

public static class SideNotchGeometry
{
    /// 生成贴边形状几何。dockEdge 指小岛贴合的屏幕边；贴边侧无圆角，
    /// 悬浮端圆角 cornerRadius，与屏幕边框交接处反向倒角 curlRadius。
    public static Geometry Build(double width, double height, DockEdge edge,
        double cornerRadius = IslandMetrics.RadiusLg, double curlRadius = IslandMetrics.NotchInset)
    {
        var geo = new StreamGeometry();
        using (var ctx = geo.Open())
        {
            switch (edge)
            {
                case DockEdge.Right: BuildRight(ctx, width, height, cornerRadius, curlRadius); break;
                case DockEdge.Left: BuildLeft(ctx, width, height, cornerRadius, curlRadius); break;
                case DockEdge.Bottom: BuildBottom(ctx, width, height, cornerRadius, curlRadius); break;
                case DockEdge.Top:
                default: BuildTop(ctx, width, height, cornerRadius, curlRadius); break;
            }
        }
        geo.Freeze();
        return geo;
    }

    private static double Clamp(double v, double lo, double hi) => Math.Max(lo, Math.Min(hi, v));

    private static void BuildTop(StreamGeometryContext ctx, double w, double h, double corner, double curl)
    {
        // 顶部贴边：上部为屏幕边框(minY)，下部为悬浮端(maxY)
        corner = Clamp(corner, 0, h / 2);
        curl = Clamp(curl, 0, Math.Min(w / 3, h - corner));
        var c = Clamp(corner, 0, (w - 2 * curl) / 2);
        var bodyLeft = curl;
        var bodyRight = w - curl;

        ctx.BeginFigure(new Point(0, 0), true, true);
        if (curl > 0)
            ctx.ArcTo(new Point(bodyLeft, curl), new Size(curl, curl), 0, false, SweepDirection.Clockwise, true, true);
        ctx.LineTo(new Point(bodyLeft, h - c), true, true);
        if (c > 0)
            ctx.ArcTo(new Point(bodyLeft + c, h), new Size(c, c), 0, false, SweepDirection.Counterclockwise, true, true);
        ctx.LineTo(new Point(bodyRight - c, h), true, true);
        if (c > 0)
            ctx.ArcTo(new Point(bodyRight, h - c), new Size(c, c), 0, false, SweepDirection.Counterclockwise, true, true);
        ctx.LineTo(new Point(bodyRight, curl), true, true);
        if (curl > 0)
            ctx.ArcTo(new Point(w, 0), new Size(curl, curl), 0, false, SweepDirection.Clockwise, true, true);
    }

    private static void BuildBottom(StreamGeometryContext ctx, double w, double h, double corner, double curl)
    {
        // 底部 = 顶部垂直镜像
        corner = Clamp(corner, 0, h / 2);
        curl = Clamp(curl, 0, Math.Min(w / 3, h - corner));
        var c = Clamp(corner, 0, (w - 2 * curl) / 2);
        var bodyLeft = curl;
        var bodyRight = w - curl;

        ctx.BeginFigure(new Point(0, h), true, true);
        if (curl > 0)
            ctx.ArcTo(new Point(bodyLeft, h - curl), new Size(curl, curl), 0, false, SweepDirection.Counterclockwise, true, true);
        ctx.LineTo(new Point(bodyLeft, c), true, true);
        if (c > 0)
            ctx.ArcTo(new Point(bodyLeft + c, 0), new Size(c, c), 0, false, SweepDirection.Clockwise, true, true);
        ctx.LineTo(new Point(bodyRight - c, 0), true, true);
        if (c > 0)
            ctx.ArcTo(new Point(bodyRight, c), new Size(c, c), 0, false, SweepDirection.Clockwise, true, true);
        ctx.LineTo(new Point(bodyRight, h - curl), true, true);
        if (curl > 0)
            ctx.ArcTo(new Point(w, h), new Size(curl, curl), 0, false, SweepDirection.Counterclockwise, true, true);
    }

    private static void BuildRight(StreamGeometryContext ctx, double w, double h, double corner, double curl)
    {
        // 右侧贴边：右侧为屏幕边框(maxX)，左侧为悬浮端(minX)
        corner = Clamp(corner, 0, w / 2);
        curl = Clamp(curl, 0, Math.Min(h / 3, w - corner));
        var c = Clamp(corner, 0, (h - 2 * curl) / 2);
        var bodyTop = curl;
        var bodyBottom = h - curl;

        ctx.BeginFigure(new Point(w, 0), true, true);
        if (curl > 0)
            ctx.ArcTo(new Point(w - curl, bodyTop), new Size(curl, curl), 0, false, SweepDirection.Clockwise, true, true);
        ctx.LineTo(new Point(c, bodyTop), true, true);
        if (c > 0)
            ctx.ArcTo(new Point(0, bodyTop + c), new Size(c, c), 0, false, SweepDirection.Clockwise, true, true);
        ctx.LineTo(new Point(0, bodyBottom - c), true, true);
        if (c > 0)
            ctx.ArcTo(new Point(c, bodyBottom), new Size(c, c), 0, false, SweepDirection.Clockwise, true, true);
        ctx.LineTo(new Point(w - curl, bodyBottom), true, true);
        if (curl > 0)
            ctx.ArcTo(new Point(w, h), new Size(curl, curl), 0, false, SweepDirection.Clockwise, true, true);
    }

    private static void BuildLeft(StreamGeometryContext ctx, double w, double h, double corner, double curl)
    {
        // 左侧 = 右侧水平镜像
        corner = Clamp(corner, 0, w / 2);
        curl = Clamp(curl, 0, Math.Min(h / 3, w - corner));
        var c = Clamp(corner, 0, (h - 2 * curl) / 2);
        var bodyTop = curl;
        var bodyBottom = h - curl;

        ctx.BeginFigure(new Point(0, 0), true, true);
        if (curl > 0)
            ctx.ArcTo(new Point(curl, bodyTop), new Size(curl, curl), 0, false, SweepDirection.Counterclockwise, true, true);
        ctx.LineTo(new Point(w - c, bodyTop), true, true);
        if (c > 0)
            ctx.ArcTo(new Point(w, bodyTop + c), new Size(c, c), 0, false, SweepDirection.Counterclockwise, true, true);
        ctx.LineTo(new Point(w, bodyBottom - c), true, true);
        if (c > 0)
            ctx.ArcTo(new Point(w - c, bodyBottom), new Size(c, c), 0, false, SweepDirection.Counterclockwise, true, true);
        ctx.LineTo(new Point(curl, bodyBottom), true, true);
        if (curl > 0)
            ctx.ArcTo(new Point(0, h), new Size(curl, curl), 0, false, SweepDirection.Counterclockwise, true, true);
    }
}
