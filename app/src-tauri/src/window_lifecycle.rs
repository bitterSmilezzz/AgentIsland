//! Native idle lease: hidden WebViews may suspend JavaScript timers.
#[derive(Default)]
pub struct WorkbenchLease { epoch: u64, pub dirty: bool }
impl WorkbenchLease {
    pub fn created(&mut self) { self.epoch += 1; self.dirty = true; }
    pub fn shown(&mut self) { self.epoch += 1; }
    pub fn hidden(&mut self) -> u64 { self.epoch += 1; self.epoch }
    pub fn can_release(&self, epoch: u64) -> bool { self.epoch == epoch && !self.dirty }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unknown_or_unsaved_drafts_are_never_released() {
        let mut lease = WorkbenchLease::default(); lease.created();
        let epoch = lease.hidden(); assert!(!lease.can_release(epoch));
        lease.dirty = false; assert!(lease.can_release(epoch));
        lease.dirty = true; assert!(!lease.can_release(epoch));
    }
    #[test]
    fn reopening_cancels_the_old_lease() {
        let mut lease = WorkbenchLease::default(); lease.created(); lease.dirty = false;
        let old = lease.hidden(); lease.shown(); assert!(!lease.can_release(old));
        let next = lease.hidden(); assert!(lease.can_release(next));
        lease.created(); assert!(!lease.can_release(next));
    }
}
