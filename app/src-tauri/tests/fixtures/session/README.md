# Session fixtures

All messages, identifiers, timestamps, commands and outputs in this directory are
synthetic. No fixture is copied from a local agent session or account.

The Claude and Codex JSONL files replay prefixes of a tool/question lifecycle;
the Cline JSON array replays the equivalent UI-message lifecycle. Unit tests also
call the public `session::probe` file entry point on these tracked files. Empty,
malformed and incomplete records verify that insufficient evidence returns no
semantic signal rather than claiming completion.

Contract references: `Sources/AgentIslandCore/AgentSessionInspector.swift`,
`Tests/AgentIslandTestsRunner/AttentionTests.swift`, and
`Tests/AgentIslandTestsRunner/MultiAgentAdvancedTests.swift`. These fixtures cover
the Rust parser's supported slice, not every upstream protocol version.
