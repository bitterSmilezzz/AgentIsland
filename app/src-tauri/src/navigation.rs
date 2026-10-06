//! 启动期深链按窗口缓存；前端排空后才切到实时事件，避免订阅前丢失。

#[derive(Default)]
struct Slot {
    ready: bool,
    pending: Vec<String>,
}

pub struct Mailbox {
    slots: [(&'static str, Slot); 3],
}

impl Default for Mailbox {
    fn default() -> Self {
        Self {
            slots: [
                ("island", Slot::default()),
                ("sidebar", Slot::default()),
                ("workbench", Slot::default()),
            ],
        }
    }
}

impl Mailbox {
    pub fn enqueue(&mut self, intent: &str) -> Vec<&'static str> {
        let mut live = Vec::new();
        for (label, slot) in &mut self.slots {
            if slot.ready {
                live.push(*label);
            } else {
                // Lazy windows may remain unopened for the lifetime of the app.
                // Retain a short ordered replay, rather than every historic navigation.
                if slot.pending.len() == 32 {
                    slot.pending.remove(0);
                }
                slot.pending.push(intent.to_owned());
            }
        }
        live
    }

    /// Deliver a local workbench navigation without touching the resident shell.
    pub fn enqueue_for(&mut self, label: &str, intent: &str) -> bool {
        let Some((_, slot)) = self.slots.iter_mut().find(|(name, _)| *name == label) else {
            return false;
        };
        if slot.ready {
            return true;
        }
        if slot.pending.len() == 32 {
            slot.pending.remove(0);
        }
        slot.pending.push(intent.to_owned());
        false
    }

    pub fn reset(&mut self, label: &str) {
        if let Some((_, slot)) = self.slots.iter_mut().find(|(name, _)| *name == label) {
            *slot = Slot::default();
        }
    }

    pub fn drain(&mut self, label: &str) -> Vec<String> {
        let Some((_, slot)) = self.slots.iter_mut().find(|(name, _)| *name == label) else {
            return Vec::new();
        };
        // 取出一批不等于启动完成：处理期间到达的意图仍需排在下一批。
        if slot.pending.is_empty() {
            slot.ready = true;
        }
        std::mem::take(&mut slot.pending)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_intents_are_ordered_and_consumed_once_per_window() {
        let mut mailbox = Mailbox::default();
        assert!(mailbox.enqueue("Expand").is_empty());
        assert!(mailbox.enqueue("Analytics").is_empty());
        for label in ["island", "sidebar", "workbench"] {
            assert_eq!(mailbox.drain(label), ["Expand", "Analytics"]);
            assert!(mailbox.drain(label).is_empty());
            assert!(mailbox.drain(label).is_empty());
        }
        assert_eq!(
            mailbox.enqueue("Collapse"),
            ["island", "sidebar", "workbench"]
        );
    }

    #[test]
    fn arrivals_during_replay_wait_for_the_next_batch() {
        let mut mailbox = Mailbox::default();
        mailbox.enqueue("Expand");
        assert_eq!(mailbox.drain("island"), ["Expand"]);
        assert!(mailbox.enqueue("Collapse").is_empty());
        assert_eq!(mailbox.drain("island"), ["Collapse"]);
        assert!(mailbox.drain("island").is_empty());
        assert_eq!(mailbox.enqueue("Toggle"), ["island"]);
        assert_eq!(mailbox.drain("sidebar"), ["Expand", "Collapse", "Toggle"]);
    }

    #[test]
    fn destroyed_window_replays_intents_after_recreation() {
        let mut mailbox = Mailbox::default();
        mailbox.drain("workbench");
        mailbox.reset("workbench");
        assert!(mailbox.enqueue("Workbench").is_empty());
        assert_eq!(mailbox.drain("workbench"), ["Workbench"]);
    }

    #[test]
    fn unopened_windows_have_a_bounded_replay() {
        let mut mailbox = Mailbox::default();
        for index in 0..1000 {
            mailbox.enqueue(&format!("Agent({index})"));
        }
        let pending = mailbox.drain("workbench");
        assert_eq!(pending.len(), 32);
        assert_eq!(pending.last().unwrap(), "Agent(999)");
    }

    #[test]
    fn task_navigation_targets_only_the_workbench_and_survives_lazy_creation() {
        let mut m = Mailbox::default();
        assert!(!m.enqueue_for("workbench", "Tasks"));
        assert!(m.drain("island").is_empty());
        assert!(m.drain("sidebar").is_empty());
        assert_eq!(m.drain("workbench"), ["Tasks"]);
        assert!(m.drain("workbench").is_empty());
        assert!(m.enqueue_for("workbench", "Tasks"));
        assert!(!m.enqueue_for("unknown", "Tasks"));
    }
    #[test]
    fn unknown_window_cannot_activate_a_real_window() {
        let mut mailbox = Mailbox::default();
        assert!(mailbox.drain("unknown").is_empty());
        assert!(mailbox.enqueue("Expand").is_empty());
        assert_eq!(mailbox.drain("island"), ["Expand"]);
    }
}
