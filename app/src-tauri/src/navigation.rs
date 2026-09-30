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
                slot.pending.push(intent.to_owned());
            }
        }
        live
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
    fn unknown_window_cannot_activate_a_real_window() {
        let mut mailbox = Mailbox::default();
        assert!(mailbox.drain("unknown").is_empty());
        assert!(mailbox.enqueue("Expand").is_empty());
        assert_eq!(mailbox.drain("island"), ["Expand"]);
    }
}
