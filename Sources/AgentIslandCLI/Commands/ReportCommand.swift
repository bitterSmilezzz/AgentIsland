import Foundation
import AppKit
import AgentIslandCore

public enum ReportCommand {
    @MainActor
    public static func run(args: [String]) async {
        let isCopy = args.contains("--copy") || args.contains("-c")

        var outputPath: String? = nil
        for (i, arg) in args.enumerated() {
            if (arg == "--output" || arg == "-o") && i + 1 < args.count {
                outputPath = args[i + 1]
                break
            }
        }

        let installedApps = InstalledAppsCache()
        installedApps.refresh()

        let engine = ActivityEngine(
            profiles: AgentRegistry.builtin,
            installedApps: installedApps
        )
        let snapshots = engine.sample()

        let md = AuditReportExporter.generateMarkdown(
            snapshots: snapshots,
            history: [],
            now: Date()
        )

        if isCopy {
            let pb = NSPasteboard.general
            pb.clearContents()
            pb.setString(md, forType: .string)
            print(CLIColor.green("已生成并复制 Markdown 审计报告至系统剪贴板。"))
            return
        }

        if let path = outputPath {
            let expandedPath = NSString(string: path).expandingTildeInPath
            do {
                try md.write(toFile: expandedPath, atomically: true, encoding: .utf8)
                print(CLIColor.green("审计报告已成功导出至: \(expandedPath)"))
            } catch {
                print(CLIColor.red("导出报告失败: \(error.localizedDescription)"))
            }
            return
        }

        // 默认打印至标准输出
        print(md)
    }
}
