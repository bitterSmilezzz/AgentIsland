import SwiftUI
import AgentIslandCore

// MARK: - 反向倒角一体化贴边形状（Side & Top Notch Shape）
// 灵感源自 CodeNotch 与 MacBook 原生硬件刘海：
// 在屏幕贴合边缘引入反向圆角（Inverse Flares / Curl Radius），使小岛仿佛从屏幕边框一体化生长出来，
// 彻底消除生硬的切边与悬浮漂移感。

struct SideNotchShape: Shape {
    var dockEdge: DockEdge = .right
    var curlRadius: CGFloat = 12
    var cornerRadius: CGFloat = Theme.radiusLg

    init(dockEdge: DockEdge = .right, curlRadius: CGFloat = 12, cornerRadius: CGFloat = Theme.radiusLg) {
        self.dockEdge = dockEdge
        self.curlRadius = curlRadius
        self.cornerRadius = cornerRadius
    }

    func path(in rect: CGRect) -> Path {
        Path(Self.cgPath(bounds: rect, dockEdge: dockEdge, cornerRadius: cornerRadius, curlRadius: curlRadius))
    }

    /// 跨 SwiftUI 与 AppKit（ShadowHostView）共用的 CGPath 核心生成函数
    static func cgPath(bounds rect: CGRect,
                       dockEdge: DockEdge,
                       cornerRadius: CGFloat = Theme.radiusLg,
                       curlRadius: CGFloat = 12) -> CGPath {
        let path = CGMutablePath()
        guard rect.width > 0, rect.height > 0 else { return path }

        switch dockEdge {
        case .right:
            // 右侧贴边：右侧为屏幕边框（maxX），左侧为悬浮端（minX）
            let wantedCorner = max(0, min(cornerRadius, rect.width / 2))
            let curl = max(0, min(curlRadius, rect.height / 3, rect.width - wantedCorner))
            let corner = max(0, min(wantedCorner, (rect.height - 2 * curl) / 2))

            let bodyTop = rect.minY + curl
            let bodyBottom = rect.maxY - curl

            // 1. 从右上角屏幕边框开始
            path.move(to: CGPoint(x: rect.maxX, y: rect.minY))

            // 2. 上部反向倒角：向内、向下弯曲进入小岛顶边
            if curl > 0 {
                path.addArc(center: CGPoint(x: rect.maxX - curl, y: rect.minY),
                            radius: curl,
                            startAngle: 0,
                            endAngle: .pi / 2,
                            clockwise: false)
            }

            // 3. 顶边直行至左上角
            path.addLine(to: CGPoint(x: rect.minX + corner, y: bodyTop))

            // 4. 左上外圆角
            if corner > 0 {
                path.addArc(center: CGPoint(x: rect.minX + corner, y: bodyTop + corner),
                            radius: corner,
                            startAngle: .pi * 1.5,
                            endAngle: .pi,
                            clockwise: true)
            }

            // 5. 左侧主边缘直行向下
            path.addLine(to: CGPoint(x: rect.minX, y: bodyBottom - corner))

            // 6. 左下外圆角
            if corner > 0 {
                path.addArc(center: CGPoint(x: rect.minX + corner, y: bodyBottom - corner),
                            radius: corner,
                            startAngle: .pi,
                            endAngle: .pi / 2,
                            clockwise: true)
            }

            // 7. 底边直行向右至反向倒角处
            path.addLine(to: CGPoint(x: rect.maxX - curl, y: bodyBottom))

            // 8. 下部反向倒角：向下、向右弯曲回归屏幕边框
            if curl > 0 {
                path.addArc(center: CGPoint(x: rect.maxX - curl, y: rect.maxY),
                            radius: curl,
                            startAngle: .pi * 1.5,
                            endAngle: .pi * 2,
                            clockwise: false)
            }

            // 9. 闭合回屏幕边框
            path.closeSubpath()

        case .top:
            // 顶部贴边：上部为屏幕边框（minY），下部为悬浮端（maxY）
            let wantedCorner = max(0, min(cornerRadius, rect.height / 2))
            let curl = max(0, min(curlRadius, rect.width / 3, rect.height - wantedCorner))
            let corner = max(0, min(wantedCorner, (rect.width - 2 * curl) / 2))

            let bodyLeft = rect.minX + curl
            let bodyRight = rect.maxX - curl

            // 1. 从左上角屏幕边框开始
            path.move(to: CGPoint(x: rect.minX, y: rect.minY))

            // 2. 左侧反向倒角：向内、向下弯曲进入小岛左边缘
            if curl > 0 {
                path.addArc(center: CGPoint(x: rect.minX, y: rect.minY + curl),
                            radius: curl,
                            startAngle: .pi * 1.5,
                            endAngle: .pi * 2,
                            clockwise: false)
            }

            // 3. 左边直行向下至左下角
            path.addLine(to: CGPoint(x: bodyLeft, y: rect.maxY - corner))

            // 4. 左下外圆角
            if corner > 0 {
                path.addArc(center: CGPoint(x: bodyLeft + corner, y: rect.maxY - corner),
                            radius: corner,
                            startAngle: .pi,
                            endAngle: .pi / 2,
                            clockwise: true)
            }

            // 5. 底边直行向右
            path.addLine(to: CGPoint(x: bodyRight - corner, y: rect.maxY))

            // 6. 右下外圆角
            if corner > 0 {
                path.addArc(center: CGPoint(x: bodyRight - corner, y: rect.maxY - corner),
                            radius: corner,
                            startAngle: .pi / 2,
                            endAngle: 0,
                            clockwise: true)
            }

            // 7. 右边直行向上至反向倒角处
            path.addLine(to: CGPoint(x: bodyRight, y: rect.minY + curl))

            // 8. 右侧反向倒角：向上、向外弯曲回归屏幕边框
            if curl > 0 {
                path.addArc(center: CGPoint(x: rect.maxX, y: rect.minY + curl),
                            radius: curl,
                            startAngle: .pi,
                            endAngle: .pi * 1.5,
                            clockwise: false)
            }

            // 9. 闭合回屏幕边框
            path.closeSubpath()
        }

        return path
    }
}
