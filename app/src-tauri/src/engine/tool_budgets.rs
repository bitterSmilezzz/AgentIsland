//! Local tool budgets consume the same sampled net tokens as the usage cards.
//! No log rescan, network request or provider attribution happens here.
use super::*;
use crate::budget::{BudgetStatus, BudgetTracker};
use serde::Serialize;
use std::path::Path;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ToolBudgetRow {
    agent_id: String,
    name: String,
    budget: i64,
    used: Option<i64>,
    source: &'static str,
    status: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ToolBudgetReport {
    rows: Vec<ToolBudgetRow>,
    alerts_enabled: bool,
}

#[derive(Default)]
pub(super) struct ToolBudgetTrackers(HashMap<String, (i64, BudgetTracker)>);

impl ToolBudgetTrackers {
    fn evaluate(
        &mut self,
        id: &str,
        budget: i64,
        used: Option<i64>,
        alerts: bool,
        now: i64,
    ) -> Option<(bool, String)> {
        let entry = self
            .0
            .entry(id.into())
            .or_insert_with(|| (budget, BudgetTracker::new()));
        if entry.0 != budget {
            *entry = (budget, BudgetTracker::new());
        }
        // Missing observations and muted alerts do not rearm or consume a crossing.
        let used = used.filter(|_| alerts)?;
        let (status, alert) = entry.1.evaluate(used, budget, now);
        alert.map(|detail| (status.is_exceeded(), detail))
    }
}

impl ActivityEngine {
    pub(crate) fn tool_budget_report(&self) -> ToolBudgetReport {
        let rows = self
            .profiles
            .iter()
            .map(|profile| {
                let budget = self
                    .settings
                    .tool_token_budgets
                    .get(&profile.id)
                    .copied()
                    .unwrap_or(0);
                let enabled = !self.settings.disabled_agents.contains(&profile.id);
                let used = enabled
                    .then(|| {
                        self.snapshots
                            .iter()
                            .find(|snapshot| snapshot.id == profile.id)
                            .and_then(|snapshot| snapshot.token_usage.as_ref())
                            .map(|usage| usage.tokens24h.max(0))
                    })
                    .flatten();
                let status = if budget == 0 {
                    "unset"
                } else if let Some(used) = used {
                    match BudgetTracker::new().evaluate(used, budget, 0).0 {
                        BudgetStatus::Exceeded { .. } => "exceeded",
                        BudgetStatus::Warning { .. } => "warning",
                        _ => "normal",
                    }
                } else {
                    "unavailable"
                };
                ToolBudgetRow {
                    agent_id: profile.id.clone(),
                    name: profile.name.clone(),
                    budget,
                    used,
                    source: if !enabled {
                        "disabled"
                    } else if used.is_some() {
                        "local"
                    } else {
                        "unavailable"
                    },
                    status,
                }
            })
            .collect();
        ToolBudgetReport {
            rows,
            alerts_enabled: self.settings.budget_alert_enabled,
        }
    }

    /// The shared engine lock serializes compare, persistence and in-memory publish.
    /// Only this tool is changed; failed writes preserve the previous state.
    pub(crate) fn set_tool_budget(
        &mut self,
        id: &str,
        budget: i64,
        expected: i64,
        dir: Option<&Path>,
    ) -> Result<(), String> {
        if !self.profiles.iter().any(|profile| profile.id == id) {
            return Err("未识别的工具，请刷新后重试".into());
        }
        if !(0..=1_000_000_000).contains(&budget) {
            return Err("预算需为 0 至 1,000,000,000 的整数".into());
        }
        let current = self
            .settings
            .tool_token_budgets
            .get(id)
            .copied()
            .unwrap_or(0);
        if current != expected {
            return Err("预算已在其他窗口修改；刷新后核对，或取消草稿读取当前值".into());
        }
        if current == budget {
            return Ok(());
        }
        if budget > 0 && current == 0 && self.settings.tool_token_budgets.len() >= 128 {
            return Err("工具预算已达容量上限，请先取消不需要的预算".into());
        }
        let mut settings = self.settings.clone();
        if budget == 0 {
            settings.tool_token_budgets.remove(id);
        } else {
            settings.tool_token_budgets.insert(id.into(), budget);
        }
        if let Some(dir) = dir {
            settings
                .try_save_to(dir)
                .map_err(|_| "预算未保存，请检查本地设置的写入权限后重试")?;
        }
        self.settings = settings;
        self.tool_budgets.0.remove(id);
        Ok(())
    }

    pub(super) fn evaluate_tool_budgets(&mut self, now: i64) {
        self.tool_budgets
            .0
            .retain(|id, _| self.settings.tool_token_budgets.contains_key(id));
        if self.settings.tool_token_budgets.is_empty() {
            return;
        }
        let mut events = Vec::new();
        for row in self.tool_budget_report().rows {
            if row.budget <= 0 {
                continue;
            }
            if let Some((exceeded, detail)) = self.tool_budgets.evaluate(
                &row.agent_id,
                row.budget,
                row.used,
                self.settings.budget_alert_enabled,
                now,
            ) {
                events.push(AgentTaskEvent {
                    id: crate::webhook::webhook_uuid(),
                    agent_id: row.agent_id,
                    agent_name: row.name,
                    event_type: "costSpike".into(),
                    timestamp: now,
                    message: Some(
                        if exceeded {
                            "工具预算超额"
                        } else {
                            "工具预算预警"
                        }
                        .into(),
                    ),
                    detail: Some(format!("本机滚动 24h 净消耗 · {detail}")),
                    duration: 0.0,
                    externally_delivered: false,
                });
            }
        }
        for event in events {
            self.push_event(event);
        }
    }
}

#[cfg(test)]
#[path = "tool_budget_tests.rs"]
mod tests;
