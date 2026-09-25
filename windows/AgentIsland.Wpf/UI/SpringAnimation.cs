using System.Windows;
using System.Windows.Media;
using System.Windows.Media.Animation;

namespace AgentIsland.UI;

/// 阻尼弹簧动画（对应 SwiftUI spring(response:dampingFraction:)）：
/// ω = 2π/response，ζ = dampingFraction。
public sealed class SpringDoubleAnimation : DoubleAnimationBase
{
    public static readonly DependencyProperty FromProperty =
        DependencyProperty.Register(nameof(From), typeof(double), typeof(SpringDoubleAnimation), new PropertyMetadata(0.0));
    public static readonly DependencyProperty ToProperty =
        DependencyProperty.Register(nameof(To), typeof(double), typeof(SpringDoubleAnimation), new PropertyMetadata(0.0));
    public static readonly DependencyProperty ResponseProperty =
        DependencyProperty.Register(nameof(Response), typeof(double), typeof(SpringDoubleAnimation), new PropertyMetadata(0.42));
    public static readonly DependencyProperty DampingProperty =
        DependencyProperty.Register(nameof(DampingFraction), typeof(double), typeof(SpringDoubleAnimation), new PropertyMetadata(0.8));

    public double From { get => (double)GetValue(FromProperty); set => SetValue(FromProperty, value); }
    public double To { get => (double)GetValue(ToProperty); set => SetValue(ToProperty, value); }
    public double Response { get => (double)GetValue(ResponseProperty); set => SetValue(ResponseProperty, value); }
    public double DampingFraction { get => (double)GetValue(DampingProperty); set => SetValue(DampingProperty, value); }

    protected override Freezable CreateInstanceCore() => new SpringDoubleAnimation
    {
        From = From,
        To = To,
        Response = Response,
        DampingFraction = DampingFraction,
    };

    protected override double GetCurrentValueCore(double defaultOriginValue, double defaultDestinationValue, AnimationClock clock)
    {
        var t = clock.CurrentTime?.TotalSeconds ?? 0;
        var omega = 2 * Math.PI / Math.Max(0.05, Response);
        var zeta = Math.Max(0.05, DampingFraction);
        var delta = From - To;
        double v;
        if (zeta < 1)
        {
            var wd = omega * Math.Sqrt(1 - zeta * zeta);
            v = To + Math.Exp(-zeta * omega * t) * delta *
                (Math.Cos(wd * t) + (zeta * omega / wd) * Math.Sin(wd * t));
        }
        else
        {
            v = To + Math.Exp(-omega * t) * delta * (1 + omega * t);
        }
        return v;
    }
}

/// 离散弹簧：把同样的弹簧物理用于任意 double 属性（如窗口尺寸），
/// 通过 CompositionTarget.Rendering 手工驱动，展开/收起用同一套物理。
public sealed class SpringDriver : IDisposable
{
    private readonly Action<double> _setter;
    private DateTime _started;
    private double _from, _to;
    private double _response, _damping;
    private bool _running;

    public SpringDriver(Action<double> setter) => _setter = setter;

    public void Animate(double from, double to, double response = 0.42, double damping = 0.8)
    {
        _from = from; _to = to;
        _response = response; _damping = damping;
        _started = DateTime.UtcNow;
        if (!_running)
        {
            _running = true;
            CompositionTarget.Rendering += OnFrame;
        }
    }

    private void OnFrame(object? sender, EventArgs e)
    {
        var t = (DateTime.UtcNow - _started).TotalSeconds;
        var omega = 2 * Math.PI / Math.Max(0.05, _response);
        var zeta = Math.Max(0.05, _damping);
        var delta = _from - _to;
        double v;
        if (zeta < 1)
        {
            var wd = omega * Math.Sqrt(1 - zeta * zeta);
            v = _to + Math.Exp(-zeta * omega * t) * delta *
                (Math.Cos(wd * t) + (zeta * omega / wd) * Math.Sin(wd * t));
        }
        else
        {
            v = _to + Math.Exp(-omega * t) * delta * (1 + omega * t);
        }
        // 2.5 秒后视为收敛（振幅足够小即提前停）
        var settled = Math.Abs(v - _to) < 0.05 && t > 0.4;
        if (t > 2.5 || settled)
        {
            v = _to;
            _running = false;
            CompositionTarget.Rendering -= OnFrame;
        }
        _setter(v);
    }

    public void Dispose()
    {
        if (_running)
        {
            _running = false;
            CompositionTarget.Rendering -= OnFrame;
        }
    }
}
