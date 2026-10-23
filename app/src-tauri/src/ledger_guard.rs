//! 账本边界守卫：**本地合计里不许混进网关配额**（GW-08「本地账本不重复相加」）。
//!
//! 口径：网关报的余额/额度是**别人给的数**，只做展示（连接页那一行），
//! 永远不并进本地 token/费用合计。两条来源相加会得到一个既不是本机用量、
//! 也不是对端账目的数——比少算更糟，因为它看起来是准的。
//!
//! 为什么扫描源码而不是做行为断言：今天根本没有「把配额加进合计」的代码路径，
//! 行为断言会**恒真**。这里要挡的是**将来有人顺手加上**，所以查的是聚合模块的引用面；
//! 并带一个正例控制，证明扫描器本身抓得到引用。

/// 在给定源码集合里找「配额」引用；返回 `文件:命中词` 列表。
pub fn aggregation_quota_leaks(sources: &[(&str, &str)]) -> Vec<String> {
    let mut leaks = Vec::new();
    for (name, source) in sources {
        for needle in ["BalanceInfo", "balance_path", "QuotaConfig", "quota_url"] {
            if source.contains(needle) {
                leaks.push(format!("{name}:{needle}"));
            }
        }
    }
    leaks
}

/// 聚合链路的源码清单。**不含本文件**：守卫自己会写下这些词，扫进来必然自命中。
pub const AGGREGATION_MODULES: &[&str] = &[
    "tokens.rs",
    "report.rs",
    "usage_context.rs",
    "usage_projection.rs",
    "budget.rs",
    "cost.rs",
];

#[cfg(test)]
mod tests {
    use super::*;

    /// 正例控制：扫描函数本身坏了（永远返回空）时，下面那条真源码断言会变成假绿。
    #[test]
    fn the_scanner_finds_a_planted_reference() {
        let planted = [("fixture-leaf.rs", "use crate::connection_probe::BalanceInfo;")];
        assert_eq!(
            aggregation_quota_leaks(&planted),
            vec!["fixture-leaf.rs:BalanceInfo".to_string()]
        );
        assert_eq!(aggregation_quota_leaks(&[("clean.rs", "fn f() {}")]), Vec::<String>::new());
    }

    #[test]
    fn the_local_ledger_never_adds_gateway_quota_into_its_totals() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let texts: Vec<(String, String)> = AGGREGATION_MODULES
            .iter()
            .map(|name| {
                let text = std::fs::read_to_string(root.join(name))
                    .unwrap_or_else(|_| panic!("读不到聚合模块 {name}"));
                ((*name).to_string(), text)
            })
            .collect();
        let borrowed: Vec<(&str, &str)> = texts
            .iter()
            .map(|(name, text)| (name.as_str(), text.as_str()))
            .collect();
        let leaks = aggregation_quota_leaks(&borrowed);
        assert!(
            leaks.is_empty(),
            "聚合链路引用了网关配额，两条来源会被相加：{leaks:?}"
        );
    }
}
