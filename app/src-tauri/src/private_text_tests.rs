//! Direct tests for known credential shapes, including boundaries and false positives.

use super::*;

fn private(text: &str) -> bool {
    known_private(text)
}

#[test]
fn known_token_prefixes_are_recognised() {
    for prefix in ["sk-", "sk_", "ghp_", "github_pat_", "AKIA", "xoxb-"] {
        let sample = format!("{prefix}{}", "a".repeat(20));
        assert!(private(&sample), "已登记前缀必须命中: {prefix}");
    }
}

#[test]
fn a_known_prefix_needs_at_least_twelve_trailing_characters() {
    let short = format!("{}{}", "sk-", "a".repeat(11));
    let minimum = format!("{}{}", "sk-", "a".repeat(12));
    assert!(!private(&short), "11 位不足门槛");
    assert!(private(&minimum), "恰 12 位应命中");
}

#[test]
fn a_prefix_must_start_on_a_boundary() {
    let token = format!("{}{}", "sk-", "a".repeat(20));
    assert!(!private(&format!("x{token}")), "字母紧贴前缀不算边界");
    assert!(!private(&format!("1{token}")), "数字紧贴前缀不算边界");
    assert!(private(&token), "行首是边界");
    assert!(private(&format!("token={token}")), "等号后是边界");
    assert!(private(&format!("see {token} here")), "空格后是边界");
}

#[test]
fn credential_assignments_are_recognised() {
    for (key, separator) in [
        ("API_KEY", "="),
        ("apikey", ": "),
        ("PASSWORD", "="),
        ("access_token", "="),
        ("client_secret", "="),
        ("MY_API_KEY", "="),
    ] {
        let line = format!("{key}{separator}{}", "a".repeat(8));
        assert!(private(&line), "凭据赋值必须命中: {key}");
    }
}

#[test]
fn env_references_and_empty_values_are_not_leaks() {
    for line in [
        "api_key=$FROM_ENV",
        "api_key=env:PROD_KEY",
        "api_key=process.env.KEY",
        "api_key=",
        "password:   ",
    ] {
        assert!(!private(line), "引用或空值不是泄露: {line}");
    }
}

#[test]
fn unrelated_keys_are_not_matched() {
    for line in [
        "apikeys_count=3",
        "password_hint=ask",
        "secret_rotation_days=30",
        "my_api_key_file=/etc/keys.list",
    ] {
        assert!(!private(line), "非凭据键不应命中: {line}");
    }
}

#[test]
fn key_material_markers_are_recognised() {
    let header = ["-----BEGIN ", "PRIVATE ", "KEY-----"].concat();
    let bearer = format!("Authorization: Bearer {}", "a".repeat(16));
    let basic = format!("authorization: Basic {}", "YQ==".repeat(3));
    assert!(private(&header));
    assert!(private(&bearer));
    assert!(private(&basic));
}

#[test]
fn credentials_embedded_in_urls_are_recognised() {
    let userinfo = format!("{}:{}", "user", "a".repeat(8));
    let url = format!("https://{userinfo}@{}/path", "example.com");
    assert!(private(&url), "URL 用户段");
    for key in ["token", "access_token", "key", "secret"] {
        let url = format!("https://example.com/api?{key}={}", "a".repeat(20));
        assert!(private(&url), "query 里的 {key}");
    }
}

#[test]
fn ordinary_urls_are_not_matched() {
    assert!(!private("https://example.com/path?page=2"), "普通分页参数");
    assert!(
        !private("https://example.com/api?token="),
        "空 token 没有内容"
    );
    assert!(!private("https://example.com/docs#section"), "锚点不是凭据");
}

#[test]
fn ordinary_text_is_not_flagged() {
    for text in [
        "",
        "just a normal sentence about work",
        "the api documentation is in docs/api.md",
        "skipping the boring parts of this diff",
        "请帮我把这个函数重命名",
        "session id: 550e8400-e29b-41d4-a716-446655440000",
    ] {
        assert!(!private(text), "正常内容不应被判成私密: {text:?}");
    }
}

#[test]
fn multiline_input_is_scanned_line_by_line_for_assignments() {
    let assignment = format!(
        "line one is fine\nline two is fine\napi_key={}\n",
        "a".repeat(8)
    );
    let token = format!("prefix text\n{}{}\nsuffix text", "sk-", "a".repeat(20));
    assert!(private(&assignment));
    assert!(private(&token));
}

#[test]
fn detection_is_case_insensitive() {
    for prefix in ["sK-", "Sk-", "gItHuB_pAt_"] {
        let sample = format!("{prefix}{}", "a".repeat(20));
        assert!(private(&sample), "混合大小写前缀必须命中: {prefix}");
    }
}
