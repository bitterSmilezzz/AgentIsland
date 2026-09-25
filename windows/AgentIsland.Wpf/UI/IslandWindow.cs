using System.Runtime.InteropServices;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Interop;
using System.Windows.Media;
using System.Windows.Media.Animation;
using System.Windows.Threading;
using AgentIsland.Core;

namespace AgentIsland.UI;

// MARK: - 监视器信息（Win32）

internal static class MonitorInfoHelper
{
    [StructLayout(LayoutKind.Sequential)]
    private struct RECT { public int Left, Top, Right, Bottom; }

    [StructLayout(LayoutKind.Sequential)]
    private struct MONITORINFO
    {
        public int cbSize;
        public RECT rcMonitor;
        public RECT rcWork;
        public uint dwFlags;
    }

    [DllImport("user32.dll")]
    private static extern bool GetCursorPos(out POINT p);

    [DllImport("user32.dll")]
    private static extern IntPtr MonitorFromPoint(POINT p, uint flags);

    [DllImport("user32.dll", CharSet = CharSet.Auto)]
    private static extern bool GetMonitorInfo(IntPtr hMonitor, ref MONITORINFO lpmi);

    [DllImport("shcore.dll")]
    private static extern uint GetDpiForMonitor(IntPtr hMonitor, int type, out uint dpiX, out uint dpiY);

    [StructLayout(LayoutKind.Sequential)]
    private struct POINT { public int X, Y; }

    public static (Rect work, double dpi) MonitorUnderCursor()
    {
        GetCursorPos(out var pt);
        var mon = MonitorFromPoint(pt, 2 /* MONITOR_DEFAULTTONEAREST */);
        return MonitorInfo(mon);
    }

    public static (Rect work, double dpi) MonitorInfo(IntPtr mon)
    {
        var mi = new MONITORINFO { cbSize = Marshal.SizeOf<MONITORINFO>() };
        double dpi = 96;
        if (GetMonitorInfo(mon, ref mi))
        {
            if (GetDpiForMonitor(mon, 0, out var dx, out _) == 0) dpi = dx;
            // Win32 返回物理像素；WPF 窗口坐标是 DIP，必须换算（高 DPI 下不换算会整体错位）
            var scale = dpi / 96.0;
            return (new Rect(mi.rcWork.Left / scale, mi.rcWork.Top / scale,
                    (mi.rcWork.Right - mi.rcWork.Left) / scale,
                    (mi.rcWork.Bottom - mi.rcWork.Top) / scale), dpi);
        }
        return (new Rect(0, 0, 1920, 1040), 96);
    }
}

// MARK: - 灵动岛窗口（NSPanel 的 Windows 对应物）
// docked：任一边贴边留 6pt 微细条（悬停弹性弹出）；
// expanded：玻璃卡片（列表 / 详情 / 会话三级导航）。

public sealed class IslandWindow : Window
{
    private readonly ActivityEngine _engine;
    private readonly SettingsStore _settings;
    
    private readonly DispatcherTimer _collapseTimer;

    private Grid _root = null!;
    private ContentControl _host = null!;

    public bool IsExpanded { get; private set; }
    private DockEdge _edge;
    private double _anchor;          // 0..1
    

    private Rect _work;
    private double _dpi = 96;
    private bool _placed;

    private string _route = "list";
    private string? _detailAgentId;
    private (string, string)? _sessionsRoute;
    private CardBuilder _card = null!;
    private SliverVisual? _sliver;

    public IslandWindow(ActivityEngine engine, SettingsStore settings)
    {
        _engine = engine;
        _settings = settings;
        _edge = settings.Edge;
        _anchor = Math.Clamp(settings.DockAnchor, 0, 1);

        WindowStyle = WindowStyle.None;
        ResizeMode = ResizeMode.NoResize;
        AllowsTransparency = true;
        Background = null!;
        ShowInTaskbar = false;
        ShowActivated = false;
        Topmost = true;
        Width = IslandMetrics.TopSliverWidth;
        Height = IslandMetrics.TopSliverHeight;
        Title = "AgentIsland";



        _collapseTimer = new DispatcherTimer { Interval = TimeSpan.FromSeconds(Math.Clamp(settings.CollapseDelay, 0.2, 30)) };
        _collapseTimer.Tick += (_, _) =>
        {
            _collapseTimer.Stop();
            if (IsExpanded && !IsMouseOver) Collapse();
        };

        Loaded += (_, _) =>
        {
            var src = (HwndSource)PresentationSource.FromVisual(this)!;
            src.AddHook(WndProc);
            _placed = false;
            RebuildVisual();
            ApplyDockedPlacement(animated: false);
        };

        MouseLeave += (_, _) => { if (IsExpanded) _collapseTimer.Start(); };
        MouseEnter += (_, _) => _collapseTimer.Stop();
        Deactivated += (_, _) => { if (IsExpanded) Collapse(); };
        ThemeManager.ThemeChanged += () => Dispatcher.BeginInvoke(RebuildVisual);
        _engine.Changed += () => Dispatcher.BeginInvoke(OnEngineChanged);
        _engine.EventRaised += e => Dispatcher.BeginInvoke(OnEventRaised, e);
    }

    // MARK: 引擎事件

    private void OnEngineChanged()
    {
        _sliver?.Update(_engine);
        if (IsExpanded && _route == "list")
        {
            var newHeight = IslandMetrics.ExpandedHeight("list", _engine.VisibleSnapshots.Length,
                !_engine.GrandTotal.Equals(default), _engine.RingShelfSnapshots.Length > 0,
                _engine.LatestEvent != null, _card?.EventExpanded ?? false);
            if (Math.Abs(Height - newHeight) > 1 && !_heightAnimating)
                AnimateWindowSize(ActualWidth, newHeight);
            _card?.RebuildList();
        }
        else if (IsExpanded && (_route == "agentDetail" || _route == "tokenAnalytics" || _route == "sessions"))
        {
            _card?.RebuildList();
        }
    }

    private void OnEventRaised(AgentTaskEvent e)
    {
        _sliver?.Update(_engine);
        // 声音（按通知分级）
        var policy = _settings.Policy;
        if (_settings.PlayCompletionSound && policy != NotificationPolicy.Silent)
        {
            var shouldPlay = e.Type switch
            {
                AgentTaskEvent.EventType.Completed => policy == NotificationPolicy.Standard,
                _ => true, // attention / costSpike 三档都出声
            };
            if (shouldPlay)
                System.Media.SystemSounds.Asterisk.Play();
        }
        if (IsExpanded) _card?.RebuildList();
    }

    // MARK: 视觉构建

    private void RebuildVisual()
    {
        _root = new Grid { Background = null! };
        _host = new ContentControl { Focusable = false };
        _root.Children.Add(_host);
        Content = _root;

        if (IsExpanded)
        {
            _card = new CardBuilder(this, _engine, _settings, Navigate, Collapse);
            _host.Content = _card.Build(_route, _detailAgentId, _sessionsRoute);
        }
        else
        {
            _sliver = new SliverVisual(_edge, () => Expand());
            _host.Content = _sliver;
        }
    }

    private void OnEngineChangedForSliver() => _sliver?.Update(_engine);

    internal void RebuildForTheme()
    {
        var wasExpanded = IsExpanded;
        RebuildVisual();
        if (wasExpanded) _card.RebuildList();
        else ApplyDockedPlacement(animated: false);
    }

    // MARK: 展开 / 收起

    public void Toggle()
    {
        if (IsExpanded) Collapse(); else Expand();
    }

    public void Expand()
    {
        if (IsExpanded) return;
        IsExpanded = true;
        _collapseTimer.Stop();
        ComputeWorkArea();

        _route = "list";
        _detailAgentId = null;
        _sessionsRoute = null;
        RebuildVisual();

        // 锚点：细条在屏幕边上的中心位置


        var targetH = IslandMetrics.ExpandedHeight("list", _engine.VisibleSnapshots.Length,
            !_engine.GrandTotal.Equals(default), _engine.RingShelfSnapshots.Length > 0,
            _engine.LatestEvent != null, false);
        ApplyPlacement();
        AnimateWindowSize(IslandMetrics.CardWidth, targetH);
        _card.PlayOpenAnimation();
    }

    public void Collapse()
    {
        if (!IsExpanded) return;
        IsExpanded = false;
        _collapseTimer.Stop();
        _route = "list";
        RebuildVisual();
        ApplyDockedPlacement(animated: true);
    }

    private bool _heightAnimating;

    private void AnimateWindowSize(double toW, double toH)
    {
        _heightAnimating = true;
        var animW = new SpringDoubleAnimation
        {
            From = ActualWidth,
            To = toW,
            Response = 0.42,
            DampingFraction = 0.8,
            Duration = new Duration(TimeSpan.FromSeconds(1.4)),
        };
        var animH = new SpringDoubleAnimation
        {
            From = ActualHeight,
            To = toH,
            Response = 0.42,
            DampingFraction = 0.8,
            Duration = new Duration(TimeSpan.FromSeconds(1.4)),
        };
        var clockW = animW.CreateClock();
        var clockH = animH.CreateClock();
        this.ApplyAnimationClock(WidthProperty, clockW);
        this.ApplyAnimationClock(HeightProperty, clockH);
        // 位置随尺寸同步（贴边放置是尺寸的函数）
        void OnTick(object? s, EventArgs e)
        {
            ApplyPlacement();
            if (Math.Abs(ActualHeight - toH) < 0.5 && Math.Abs(ActualWidth - toW) < 0.5)
            {
                clockH.CurrentTimeInvalidated -= OnTick;
                clockW.CurrentTimeInvalidated -= OnTick;
                this.BeginAnimation(WidthProperty, null);
                this.BeginAnimation(HeightProperty, null);
                Width = toW;
                Height = toH;
                _heightAnimating = false;
            }
        }
        clockH.CurrentTimeInvalidated += OnTick;
        clockW.CurrentTimeInvalidated += OnTick;
        ApplyPlacement();
    }

    // MARK: 放置逻辑
    // 统一锚点模型：细条中心 = 卡片中心 = _anchor × 工作区（水平边沿 X，垂直边沿 Y）。

    private void ComputeWorkArea()
    {
        var (work, dpi) = MonitorInfoHelper.MonitorUnderCursor();
        _work = work;
        _dpi = dpi;
    }

    private void EnsureWorkArea()
    {
        if (_placed) return;
        ComputeWorkArea();
        _placed = true;
    }

    private (double w, double h) DockedVisualSize()
    {
        // 视觉 6pt + 命中区 12pt（命中区补的是可达性，不是画出来的尺寸）
        return _edge.IsHorizontal()
            ? (IslandMetrics.TopSliverWidth, IslandMetrics.TopSliverHeight + 12)
            : (IslandMetrics.RightSliverWidth + 12, IslandMetrics.RightSliverHeight);
    }

    /// 按当前 ActualWidth/Height + 贴边锚点放置（动画过程中的每一帧也走这里）
    private void ApplyPlacement()
    {
        EnsureWorkArea();
        var w = ActualWidth;
        var h = ActualHeight;
        double left, top;
        if (_edge.IsHorizontal())
        {
            var centerX = _work.Left + _anchor * _work.Width;
            left = Math.Max(_work.Left, Math.Min(centerX - w / 2, _work.Right - w));
            top = _edge == DockEdge.Top ? _work.Top : _work.Bottom - h;
        }
        else
        {
            var centerY = _work.Top + _anchor * _work.Height;
            top = Math.Max(_work.Top, Math.Min(centerY - h / 2, _work.Bottom - h));
            left = _edge == DockEdge.Left ? _work.Left : _work.Right - w;
        }
        Left = left;
        Top = top;
    }

    private void ApplyDockedPlacement(bool animated)
    {
        EnsureWorkArea();
        var (w, h) = DockedVisualSize();
        if (animated)
        {
            AnimateWindowSize(w, h);
        }
        else
        {
            Width = w;
            Height = h;
            ApplyPlacement();
        }
    }

    /// 拖拽释放后吸附到最近边并持久化
    internal void SnapToNearestEdge()
    {
        ComputeWorkArea();
        var centerX = Left + ActualWidth / 2;
        var centerY = Top + ActualHeight / 2;
        var dTop = centerY - _work.Top;
        var dBottom = _work.Bottom - centerY;
        var dLeft = centerX - _work.Left;
        var dRight = _work.Right - centerX;
        var min = Math.Min(Math.Min(dTop, dBottom), Math.Min(dLeft, dRight));
        _edge = min switch
        {
            _ when min == dTop => DockEdge.Top,
            _ when min == dBottom => DockEdge.Bottom,
            _ when min == dLeft => DockEdge.Left,
            _ => DockEdge.Right,
        };
        if (_edge.IsHorizontal())
            _anchor = Math.Clamp((centerX - _work.Left) / Math.Max(1, _work.Width - IslandMetrics.CardWidth), 0, 1);
        else
            _anchor = Math.Clamp((centerY - _work.Top) / Math.Max(1, _work.Height - IslandMetrics.CardWidth), 0, 1);
        _settings.DockEdge = _edge.ToString();
        _settings.DockAnchor = _anchor;
        _settings.Save();
        RebuildVisual();
        ApplyPlacement();
    }

    internal void SetDockEdge(DockEdge edge)
    {
        _edge = edge;
        _settings.DockEdge = edge.ToString();
        _settings.Save();
        RebuildVisual();
        if (IsExpanded) ApplyPlacement();
        else ApplyDockedPlacement(false);
    }

    internal void SetCollapseDelay(double seconds)
    {
        _collapseTimer.Interval = TimeSpan.FromSeconds(Math.Clamp(seconds, 0.2, 30));
    }

    internal void ApplyAppearance(IslandAppearance mode)
    {
        _settings.Appearance = mode.ToString();
        _settings.Save();
        ThemeManager.Apply(mode);
    }

    internal void Navigate(string route, string? agentId = null, (string, string)? sessions = null)
    {
        _route = route;
        _detailAgentId = agentId;
        _sessionsRoute = sessions;
        RebuildVisual();
        var targetH = IslandMetrics.ExpandedHeight(route, _engine.VisibleSnapshots.Length,
            !_engine.GrandTotal.Equals(default), false, _engine.LatestEvent != null, false);
        AnimateWindowSize(IslandMetrics.CardWidth, targetH);
        ApplyPlacement();
    }

    /// 引擎数据变化后整卡刷新（保留路由与状态）
    internal void RefreshExpandedCard(bool force = false)
    {
        if (!IsExpanded) return;
        if (!force && _route != "list") { /* 子页也随数据轻刷新 */ }
        var route = _route;
        var agentId = _detailAgentId;
        var sessions = _sessionsRoute;
        _card = new CardBuilder(this, _engine, _settings, Navigate, Collapse);
        _card.EventExpanded = EventExpanded;
        _host.Content = _card.Build(route, agentId, sessions);
        var targetH = IslandMetrics.ExpandedHeight(route, _engine.VisibleSnapshots.Length,
            !_engine.GrandTotal.Equals(default), _engine.RingShelfSnapshots.Length > 0 && route == "list",
            _engine.LatestEvent != null && route == "list", false);
        if (Math.Abs(ActualHeight - targetH) > 1 && !_heightAnimating)
            AnimateWindowSize(IslandMetrics.CardWidth, targetH);
    }

    internal bool EventExpanded { get; set; }

    // MARK: WndProc（多屏变化 / 主题）

    private IntPtr WndProc(IntPtr hwnd, int msg, IntPtr wParam, IntPtr lParam, ref bool handled)
    {
        const int WM_DISPLAYCHANGE = 0x007E;
        const int WM_SETTINGCHANGE = 0x001A;
        if (msg is WM_DISPLAYCHANGE or WM_SETTINGCHANGE)
        {
            _placed = false;
            Dispatcher.BeginInvoke(() =>
            {
                if (IsExpanded) { ComputeWorkArea(); ApplyPlacement(); }
                else ApplyDockedPlacement(false);
            });
        }
        return IntPtr.Zero;
    }

    internal DockEdge CurrentEdge => _edge;
    internal string CurrentRoute => _route;
    internal string? DetailAgentId => _detailAgentId;
    internal (string, string)? SessionsRoute => _sessionsRoute;
}

// MARK: - 收起态微细条（DockedSliverCapsule：晶莹胶囊 + 呼吸微光）

internal sealed class SliverVisual : Grid
{
    private readonly DockEdge _edge;
    private readonly Border _capsule;
    private readonly DoubleAnimation _breathe;
    private AnimationClock? _clock;

    public SliverVisual(DockEdge edge, Action onActivate)
    {
        _edge = edge;
        var t = ThemeManager.Current;
        var horizontal = edge.IsHorizontal();

        // 整窗命中区（含向屏幕内侧的命中延伸）
        Background = Brushes.Transparent;
        MinHeight = horizontal ? IslandMetrics.TopSliverHeight + 12 : IslandMetrics.RightSliverHeight;
        MinWidth = horizontal ? IslandMetrics.TopSliverWidth : IslandMetrics.RightSliverWidth + 12;

        var w = horizontal ? IslandMetrics.TopSliverWidth : IslandMetrics.RightSliverWidth;
        var h = horizontal ? IslandMetrics.TopSliverHeight : IslandMetrics.RightSliverHeight;
        _capsule = new Border
        {
            Width = w,
            Height = h,
            CornerRadius = new CornerRadius(Math.Min(w, h) / 2),
            Background = new SolidColorBrush(t.SliverFill(false, false)),
            BorderBrush = new SolidColorBrush(t.SliverStroke(false, false)),
            BorderThickness = new Thickness(0.75),
            Opacity = 0.92,
        };
        switch (edge)
        {
            case DockEdge.Top: _capsule.VerticalAlignment = VerticalAlignment.Top; break;
            case DockEdge.Bottom: _capsule.VerticalAlignment = VerticalAlignment.Bottom; break;
            case DockEdge.Left: _capsule.HorizontalAlignment = HorizontalAlignment.Left; break;
            case DockEdge.Right: _capsule.HorizontalAlignment = HorizontalAlignment.Right; break;
        }
        Children.Add(_capsule);

        MouseEnter += (_, _) => onActivate();
        MouseLeftButtonUp += (_, _) => onActivate();

        // 呼吸微光（工作态/告警态才跑；独立动画时钟，零布局开销）
        _breathe = new DoubleAnimation(0.35, 1.0, TimeSpan.FromSeconds(2.2))
        {
            AutoReverse = true,
            RepeatBehavior = RepeatBehavior.Forever,
            EasingFunction = new QuadraticEase { EasingMode = EasingMode.EaseInOut },
        };
    }

    public void Update(ActivityEngine engine)
    {
        var t = ThemeManager.Current;
        var working = engine.AnyWorking;
        var alert = engine.HasAttention || engine.LatestEvent is { Type: AgentTaskEvent.EventType.CostSpike };
        _capsule.Background = new SolidColorBrush(t.SliverFill(working, alert));
        _capsule.BorderBrush = new SolidColorBrush(t.SliverStroke(working, alert));
        _breathe.Duration = TimeSpan.FromSeconds(alert ? 1.2 : 2.2);

        var animated = alert || working;
        if (animated && _clock == null)
        {
            _clock = _breathe.CreateClock(); // DoubleAnimation.CreateClock() → AnimationClock
            _capsule.ApplyAnimationClock(UIElement.OpacityProperty, _clock);
        }
        else if (!animated && _clock != null)
        {
            _capsule.ApplyAnimationClock(UIElement.OpacityProperty, null);
            _clock = null;
            _capsule.Opacity = 0.92;
        }
    }
}

// MARK: - 卡片内容构建器（IslandView 的 Windows 对应物，见 CardViews.cs）

internal delegate void NavigateHandler(string route, string? agentId = null, (string, string)? sessions = null);
