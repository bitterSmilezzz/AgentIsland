import Foundation

// MARK: - 实时日志错误智能模式识别 (v0.0.75)

public struct AnalyzedLogIssue: Equatable, Sendable {
    public enum IssueKind: String, Codable, Equatable, Sendable {
        case compileError   // 编译错误 (Swift / TS / Rust / C++)
        case rateLimit      // API 429 速率限制 / Quota 耗尽
        case gitConflict    // Git merge conflict / push 失败
        case authError      // 鉴权失败 (401 / 403 / API Key 失效)
        case runtimeError   // 运行时异常 / Panic / Exception

        public var label: String {
            switch self {
            case .compileError: return "编译错误"
            case .rateLimit: return "API 429限流"
            case .gitConflict: return "Git冲突"
            case .authError: return "鉴权失败"
            case .runtimeError: return "异常抛错"
            }
        }
    }

    public let kind: IssueKind
    public let summary: String
    public let snippet: String

    public init(kind: IssueKind, summary: String, snippet: String) {
        self.kind = kind
        self.summary = summary
        self.snippet = snippet
    }
}

public enum LogPatternAnalyzer {

    public static func analyze(title: String, detail: String?) -> AnalyzedLogIssue? {
        let combined = "\(title)\n\(detail ?? "")"
        let lower = combined.lowercased()

        // 1. 速率限制 / 配额超限
        if lower.contains("429") || lower.contains("rate limit") || lower.contains("quota exceeded") || lower.contains("insufficient_quota") || lower.contains("resource has been exhausted") {
            let snippet = extractSnippet(from: combined, matching: ["429", "rate limit", "quota", "exhausted"])
            return AnalyzedLogIssue(kind: .rateLimit, summary: "API 调用受限或 Token 配额耗尽 (429)", snippet: snippet)
        }

        // 2. Git 冲突
        if lower.contains("conflict") || lower.contains("merge conflict") || lower.contains("failed to push") || lower.contains("non-fast-forward") {
            let snippet = extractSnippet(from: combined, matching: ["conflict", "push", "merge"])
            return AnalyzedLogIssue(kind: .gitConflict, summary: "检测到 Git 分支冲突或推送拒绝", snippet: snippet)
        }

        // 3. 鉴权失败
        if lower.contains("401 unauthorized") || lower.contains("invalid api key") || lower.contains("invalid_api_key") || lower.contains("authentication failed") || lower.contains("permission denied") {
            let snippet = extractSnippet(from: combined, matching: ["401", "unauthorized", "api key", "permission denied"])
            return AnalyzedLogIssue(kind: .authError, summary: "服务鉴权失败或缺少访问权限", snippet: snippet)
        }

        // 4. 编译错误 (Swift / TS / Rust / clang)
        if lower.contains("error:") || lower.contains("fatal error") || lower.contains("cannot find type") || lower.contains("failed with a nonzero exit code") || lower.contains("syntaxerror") || lower.contains("typeerror") {
            let snippet = extractSnippet(from: combined, matching: ["error:", "fatal", "cannot find", "nonzero", "syntaxerror"])
            return AnalyzedLogIssue(kind: .compileError, summary: "代码编译构建或语法检查失败", snippet: snippet)
        }

        // 5. 运行时异常
        if lower.contains("panic:") || lower.contains("traceback (most recent call last)") || lower.contains("uncaught exception") || lower.contains("segmentation fault") {
            let snippet = extractSnippet(from: combined, matching: ["panic", "traceback", "exception", "segmentation fault"])
            return AnalyzedLogIssue(kind: .runtimeError, summary: "运行时发生未捕获异常或 Panic", snippet: snippet)
        }

        return nil
    }

    private static func extractSnippet(from text: String, matching keywords: [String]) -> String {
        let lines = text.components(separatedBy: .newlines)
        for line in lines {
            let l = line.lowercased()
            if keywords.contains(where: { l.contains($0) }) {
                let trimmed = line.trimmingCharacters(in: .whitespacesAndNewlines)
                if !trimmed.isEmpty {
                    return String(trimmed.prefix(120))
                }
            }
        }
        return String(text.prefix(100))
    }
}
