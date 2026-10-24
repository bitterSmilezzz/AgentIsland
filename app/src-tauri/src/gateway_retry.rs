//! 网关错误的**重试资格判据表**（GW-06 首片：错误分类与退避口径）。
//!
//! 需求原文：「账号/模型/请求错误分开；首字节前重试，流中失败不拼接回答
//! 或重复工具/计费」。转发层（GW-03）还没建，但**判据必须先于实现**定死：
//! 等转发层落地时再定「什么能重试」，每个新接入的协议都会各定一套。
//!
//! 这张表只回答一个问题：**某类失败发生在首字节之前时，自动重试有没有意义？**
//! 三种答案：有意义（链路抖动/服务过载）、只按对端指示等（限流）、无意义
//! （换不来权限/结果，重试只是加负载）。判据是纯函数，不碰网络、不读时钟。
//!
//! **流中失败不在此表**：首字节已到、部分内容已交给调用方之后，任何失败都
//! **不可自动重试**——重试会拼接两份回答、重复计费与工具调用（需求明文）。
//! 那条属于转发层的流式状态机，不在本判据表里冒充覆盖。

use crate::connection_http::Failure;

/// 重试资格。注意这不是「错误类型」——同一 HTTP 状态在不同语义下资格不同
/// （429 是「稍后再来」，401 是「换凭据才有救」），资格表只看**重试能否改变结果**。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Eligibility {
    /// 链路或服务侧抖动：换等价端点重试有意义。有界（见 `budget`）。
    Transient,
    /// 服务限流：重试资格由**对端指示**决定（Retry-After），不自行猜测节奏。
    Throttled,
    /// 账号/凭据/权限类：重试换不来权限。停下报人，不重试。
    Credential,
    /// 请求或模型侧（4xx 语义错误、模型不存在、响应坏格式）：请求本身有问题，
    /// 原样重发必然原样失败。
    Request,
    /// 配置错（地址无效/明文远端）：不重试，修正配置才是出路。
    Configuration,
}

impl Eligibility {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Transient => "transient",
            Self::Throttled => "throttled",
            Self::Credential => "credential",
            Self::Request => "request",
            Self::Configuration => "configuration",
        }
    }

    /// 首字节前自动重试是否有意义（限流除外——它由对端指示决定，见模块注释）。
    pub fn auto_retryable(self) -> bool {
        matches!(self, Self::Transient)
    }
}

/// 账号/模型/请求/服务/网络五类分开的判据表。
///
/// `HttpStatus(code)` 在 [`Failure`] 里是一个桶，这里按语义拆开：
/// 401/403 归账号（连刷新失败 `RefreshFailed` 也是——刷新失败说明凭据链路坏了）；
/// 404/400/413/414/422 归请求/模型侧；429 归限流；5xx 归服务侧抖动。
pub fn classify(failure: &Failure) -> Eligibility {
    match failure {
        // 链路抖动：换等价端点重试有意义
        Failure::Timeout | Failure::Unavailable | Failure::Tls | Failure::Redirect => {
            Eligibility::Transient
        }
        // 服务过载/网关错误：服务侧抖动，有界重试
        Failure::HttpStatus(500..=599) => Eligibility::Transient,
        // 限流：对端说了什么时候再来
        Failure::RateLimited | Failure::HttpStatus(429) => Eligibility::Throttled,
        // 账号/凭据/权限：重试换不来权限。401/403 通常在传输层已折成
        // AuthFailed/Forbidden，这里防御直传状态码的路径——两类说法必须同判。
        Failure::AuthFailed | Failure::RefreshFailed | Failure::Forbidden
        | Failure::HttpStatus(401 | 403) => Eligibility::Credential,
        // 请求/模型侧：原样重发必然原样失败
        Failure::HttpStatus(400 | 404 | 405 | 413 | 414 | 415 | 422)
        | Failure::InvalidJson
        | Failure::UnsupportedEncoding
        | Failure::TooLarge => Eligibility::Request,
        // 配置错：修配置，不重试
        Failure::InvalidAddress | Failure::InsecureTransport | Failure::InvalidCredential => {
            Eligibility::Configuration
        }
        // 凭据来源不可用：环境问题，不是请求问题；修正后**手动**重试
        Failure::CredentialUnavailable => Eligibility::Configuration,
        // 未逐一列名的 4xx：请求侧兜底——请求换不来别的结果。
        // 新语义（如 408 超时想归链路）必须显式列名并配用例，不许静默漂。
        Failure::HttpStatus(_) => Eligibility::Request,
    }
}

/// 有界重试预算：Transient 在首字节前最多再试 2 次（共 3 次尝试）。
/// 与 NOT-05 的「连续 3 次临时失败升级通道故障」同一节奏：重试有界、
/// 失败可见，不无限循环。
pub fn budget(eligibility: Eligibility, prior_retries: u32) -> bool {
    eligibility.auto_retryable() && prior_retries < 2
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_model_and_request_errors_are_told_apart() {
        // 账号：401/403/刷新失败——重试换不来权限
        for failure in [
            Failure::AuthFailed,
            Failure::RefreshFailed,
            Failure::Forbidden,
        ] {
            assert_eq!(classify(&failure), Eligibility::Credential, "{failure:?}");
        }
        // 请求/模型侧：400/404/422/坏响应——原样重发必然失败
        for failure in [
            Failure::HttpStatus(400),
            Failure::HttpStatus(404),
            Failure::HttpStatus(422),
            Failure::InvalidJson,
        ] {
            assert_eq!(classify(&failure), Eligibility::Request, "{failure:?}");
        }
        // 服务/网络：500/502/超时——有界重试有意义
        for failure in [Failure::HttpStatus(502), Failure::Timeout, Failure::Unavailable] {
            assert_eq!(classify(&failure), Eligibility::Transient, "{failure:?}");
        }
        // 限流单列：资格由对端指示决定，不自行猜测节奏
        assert_eq!(classify(&Failure::RateLimited), Eligibility::Throttled);
        assert_eq!(classify(&Failure::HttpStatus(429)), Eligibility::Throttled);
        // 配置错：不重试
        assert_eq!(classify(&Failure::InvalidAddress), Eligibility::Configuration);
        assert_eq!(
            classify(&Failure::InsecureTransport),
            Eligibility::Configuration
        );
    }

    #[test]
    fn only_transient_errors_earn_a_bounded_retry_budget() {
        assert!(budget(Eligibility::Transient, 0));
        assert!(budget(Eligibility::Transient, 1));
        assert!(!budget(Eligibility::Transient, 2), "重试必须有界（共 3 次尝试）");
        // 其余类型一次都不给：自动重试只会加负载，不改变结果
        for other in [
            Eligibility::Throttled,
            Eligibility::Credential,
            Eligibility::Request,
            Eligibility::Configuration,
        ] {
            assert!(!budget(other, 0), "{other:?} 不得自动重试");
        }
    }

    #[test]
    fn every_http_status_has_a_home_and_none_is_silent() {
        // HttpStatus 桶里的每个状态码都必须落进明确的一类——
        // 「没写到的状态」按 4xx/5xx 兜底也必须有类，不许出现未分类。
        for status in [
            400u16, 401, 403, 404, 405, 408, 409, 413, 414, 415, 422, 429, 500, 502, 503, 504,
        ] {
            let eligibility = classify(&Failure::HttpStatus(status));
            // 兜底语义：其余 4xx 归请求侧（请求换不来别的结果），其余 5xx 归服务侧
            let expected = if status == 429 {
                Eligibility::Throttled
            } else if status >= 500 {
                Eligibility::Transient
            } else if status == 401 || status == 403 {
                Eligibility::Credential
            } else {
                Eligibility::Request
            };
            assert_eq!(eligibility, expected, "HTTP {status} 落类必须确定");
        }
    }
}
