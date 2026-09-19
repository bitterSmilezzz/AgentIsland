import Foundation
import AppKit
import AgentIslandCore

public enum ReportCommand {
    @MainActor
    public static func run(args: [String]) async {
        let isCopy = args.contains("--copy") || args.contains("-c")

        var outputPath: String? = nil
        var format = "markdown"

        var i = 0
        while i < args.count {
            let arg = args[i]
            if (arg == "--output" || arg == "-o") && i + 1 < args.count {
                outputPath = args[i + 1]
                i += 2
                continue
            }
            if (arg == "--format" || arg == "-f") && i + 1 < args.count {
                format = args[i + 1].lowercased()
                i += 2
                continue
            }
            i += 1
        }

        let installedApps = InstalledAppsCache()
        installedApps.refresh()

        let registry = AgentRegistry.fullRegistry(
            installedCLIs: installedApps.installedCLIs(),
            installedBundles: installedApps.installedBundleIDs()
        )
        let engine = ActivityEngine(
            profiles: registry,
            installedApps: installedApps
        )
        let snapshots = engine.sample()

        let content: String
        switch format {
        case "csv":
            content = AuditReportExporter.generateCSV(snapshots: snapshots)
        case "json":
            let dtos = snapshots.map { CLIAgentStatusDTO(from: $0) }
            let encoder = JSONEncoder()
            encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
            if let data = try? encoder.encode(dtos), let str = String(data: data, encoding: .utf8) {
                content = str
            } else {
                content = "[]"
            }
        case "raycast":
            content = AuditReportExporter.generateRaycastManifest(snapshots: snapshots)
        default:
            content = AuditReportExporter.generateMarkdown(
                snapshots: snapshots,
                history: [],
                now: Date()
            )
        }

        if isCopy {
            let pb = NSPasteboard.general
            pb.clearContents()
            pb.setString(content, forType: .string)
            print(CLIColor.green("已生成并复制 \(format.uppercased()) 报表至系统剪贴板。"))
            return
        }

        if let path = outputPath {
            let expandedPath = NSString(string: path).expandingTildeInPath
            do {
                try content.write(toFile: expandedPath, atomically: true, encoding: .utf8)
                print(CLIColor.green("审计报表 (\(format.uppercased())) 已成功导出至: \(expandedPath)"))
            } catch {
                print(CLIColor.red("导出报表失败: \(error.localizedDescription)"))
            }
            return
        }

        // 默认打印至标准输出
        print(content)
    }
}
