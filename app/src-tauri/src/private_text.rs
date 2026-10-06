//! Known credential shapes only; never a claim of universal secret detection.
pub(crate) fn known_private(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let token_prefix = ["sk-", "sk_", "ghp_", "github_pat_", "akia", "xoxb-"]
        .iter()
        .any(|prefix| {
            lower.match_indices(prefix).any(|(offset, _)| {
                let boundary = offset == 0 || !lower.as_bytes()[offset - 1].is_ascii_alphanumeric();
                boundary
                    && lower[offset + prefix.len()..]
                        .bytes()
                        .take_while(|c| c.is_ascii_alphanumeric() || *c == b'_' || *c == b'-')
                        .count()
                        >= 12
            })
        });
    let assignment = lower.lines().any(|line| {
        let Some((key, value)) = line.split_once('=').or_else(|| line.split_once(':')) else {
            return false;
        };
        let key = key
            .trim()
            .trim_matches(|c: char| c == '"' || c == '\'' || c == '`');
        let value = value
            .trim()
            .trim_matches(|c: char| c == '"' || c == '\'' || c == '`');
        ["api_key", "apikey", "password", "access_token", "secret"]
            .iter()
            .any(|name| key == *name || key.ends_with(&format!("_{name}")))
            && !value.is_empty()
            && !value.starts_with('$')
            && !value.starts_with("env:")
            && !value.starts_with("process.env")
    });
    token_prefix
        || assignment
        || lower.contains("private key")
        || lower.contains("authorization:")
        || lower.contains("bearer ")
        || text
            .split_whitespace()
            .filter_map(|part| url::Url::parse(part).ok())
            .any(|url| {
                !url.username().is_empty()
                    || url.password().is_some()
                    || url.query_pairs().any(|(key, value)| {
                        ["token", "access_token", "key", "secret"].contains(&key.as_ref())
                            && !value.is_empty()
                    })
            })
}
