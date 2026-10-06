//! Explicit opt-in lifecycle: configuration proof, private receiver, and a shared ephemeral cache.
#[cfg(target_os = "macos")]
mod native {
    use crate::{
        claude_hook_config as config, claude_plan_capture as capture,
        claude_plan_receiver as receiver,
    };
    use serde::Serialize;
    use std::path::PathBuf;
    #[derive(Serialize)]
    pub(crate) struct Status {
        pub state: &'static str,
        pub configuration_current: bool,
        pub resume_ready: bool,
        pub notice: &'static str,
        pub configuration: Option<config::Inspection>,
        pub received: u64,
        pub rejected: u64,
        pub last_received_ms: Option<i64>,
    }
    pub(crate) struct Runtime {
        config: config::native::Store,
        endpoint: PathBuf,
        roots: Vec<PathBuf>,
        receiver: Option<receiver::native::Receiver>,
        binding: Option<String>,
        state: &'static str,
        notice: &'static str,
    }
    impl Runtime {
        pub(crate) fn new(
            config: config::native::Store,
            endpoint: PathBuf,
            roots: Vec<PathBuf>,
        ) -> Self {
            Self {
                config,
                endpoint,
                roots,
                receiver: None,
                binding: None,
                state: "paused",
                notice: "采集未启动。",
            }
        }
        pub(crate) fn at_default() -> Result<Self, config::Error> {
            let home = dirs::home_dir().ok_or(config::Error {
                code: "missing",
                notice: "用户目录不可用。",
                applied: false,
            })?;
            Ok(Self::new(
                config::native::Store::at_default()?,
                receiver::default_root(),
                vec![home.join(".claude/projects"), home.join(".claude/sessions")],
            ))
        }
        fn stop(&mut self, state: &'static str, notice: &'static str) {
            if let Some(mut receiver) = self.receiver.take() {
                receiver.stop();
            }
            self.binding = None;
            self.state = state;
            self.notice = notice;
        }
        fn start(&mut self, revision: String) -> Result<(), config::Error> {
            self.stop("paused", "采集未启动。");
            let receiver = receiver::native::Receiver::start_guarded(
                &self.endpoint,
                self.roots.clone(),
                std::sync::Arc::new(self.config.lifecycle_check(revision.clone())),
            )
            .map_err(|_| config::Error {
                code: "receiver",
                notice: "接收服务未启动，请核对采集状态。",
                applied: false,
            })?;
            self.receiver = Some(receiver);
            self.binding = Some(revision);
            self.state = "running";
            self.notice = "只读采集中，正文临时保留。";
            Ok(())
        }
        pub(crate) fn resume_saved(&mut self) -> Result<(), config::Error> {
            let preference = match self.config.runtime_preference() {
                Ok(value) => value,
                Err(error) => {
                    self.stop("unavailable", "启动设置无法核实，采集未启动。");
                    return Err(error);
                }
            };
            let Some(preference) = preference else {
                return Ok(());
            };
            if !preference.enabled {
                return Ok(());
            }
            let proof = self.config.runtime_proof()?;
            if !proof.eligible || proof.revision != preference.binding {
                self.stop("changed", "配置已变化，采集暂停；请重新核对。");
                return Ok(());
            }
            self.start(proof.revision)
        }
        pub(crate) fn resume(&mut self, revision: &str) -> Result<(), config::Error> {
            let proof = self.config.runtime_proof()?;
            if !proof.eligible || proof.revision != revision {
                return Err(config::Error {
                    code: "changed",
                    notice: "配置未就绪或已变化，请重新读取。",
                    applied: false,
                });
            }
            self.config.set_runtime_preference(true, revision)?;
            if self.start(proof.revision).is_err() {
                let _ = self.config.set_runtime_preference(false, revision);
                self.stop("unavailable", "接收服务未启动，请重新核对。");
                return Err(config::Error {
                    code: "receiver",
                    notice: "配置已保留，接收服务未启动。",
                    applied: true,
                });
            }
            Ok(())
        }
        pub(crate) fn pause(&mut self) -> Result<(), config::Error> {
            self.stop("paused", "采集已暂停，临时正文已清空。");
            let result = self
                .config
                .runtime_proof()
                .and_then(|proof| self.config.set_runtime_preference(false, &proof.revision));
            if result.is_err() {
                self.state = "uncertain";
                self.notice = "本次采集已停，重启设置未核实。";
                return Err(config::Error {
                    code: "uncertain",
                    notice: self.notice,
                    applied: true,
                });
            }
            Ok(())
        }
        fn guard(&mut self) -> bool {
            let Some(binding) = self.binding.as_ref() else {
                return false;
            };
            let valid = self
                .config
                .runtime_proof()
                .is_ok_and(|p| p.eligible && p.revision == *binding)
                && self
                    .config
                    .runtime_preference()
                    .is_ok_and(|p| p.is_some_and(|p| p.enabled && p.binding == *binding))
                && self
                    .receiver
                    .as_ref()
                    .is_some_and(|r| r.status().is_ok_and(|s| s.running));
            if !valid {
                self.stop("changed", "配置或接收服务已变化，采集暂停；请重新核对。");
            }
            valid
        }
        pub(crate) fn with_cache<T>(
            &mut self,
            f: impl FnOnce(Option<&mut capture::Store>) -> T,
        ) -> T {
            if !self.guard() {
                return f(None);
            }
            let cache = self.receiver.as_ref().unwrap().cache();
            let guard = match cache.lock() {
                Ok(guard) => Some(guard),
                Err(error) => {
                    drop(error);
                    None
                }
            };
            if let Some(mut guard) = guard {
                f(Some(&mut guard))
            } else {
                self.stop("unavailable", "临时正文不可用，采集已暂停。");
                f(None)
            }
        }
        pub(crate) fn status(&mut self) -> Status {
            self.guard();
            let inspection = self.config.observe();
            let unavailable_notice = inspection.as_ref().err().map(|e| e.notice);
            let configuration = inspection.ok();
            let configuration_current = self.config.runtime_proof().is_ok_and(|p| p.eligible);
            let resume_ready = configuration_current && self.config.runtime_preference().is_ok();
            let received = self.receiver.as_ref().and_then(|r| r.status().ok());
            Status {
                configuration_current,
                resume_ready,
                state: if configuration.is_none() {
                    "unavailable"
                } else {
                    self.state
                },
                notice: if configuration.is_none() {
                    unavailable_notice.unwrap_or("配置状态无法核实，请检查来源工具。")
                } else {
                    self.notice
                },
                configuration,
                received: received.as_ref().map_or(0, |s| s.received),
                rejected: received.as_ref().map_or(0, |s| s.rejected),
                last_received_ms: received.and_then(|s| s.last_received_ms),
            }
        }
        pub(crate) fn preview(
            &mut self,
            action: config::Action,
            revision: &str,
            backup: Option<&str>,
        ) -> Result<config::Preview, config::Error> {
            self.config.preview(action, revision, backup)
        }
        pub(crate) fn cancel(&mut self) {
            self.config.cancel();
        }
        pub(crate) fn apply(&mut self, id: &str) -> Result<config::Applied, config::Error> {
            let action = self.config.preflight(id)?;
            if action == config::Action::TrashBackup {
                return self.config.apply(id);
            }
            self.stop("paused", "配置操作中，采集已暂停。");
            let result = match self.config.apply(id) {
                Ok(result) => result,
                Err(error) => {
                    if error.applied {
                        self.state = "uncertain";
                        self.notice = error.notice;
                    }
                    return Err(error);
                }
            };
            let proof = self.config.runtime_proof()?;
            self.config
                .set_runtime_preference(false, &proof.revision)
                .map_err(|_| config::Error {
                    code: "uncertain",
                    notice: "配置已核实，采集重启设置未核实。",
                    applied: true,
                })?;
            self.notice = "配置已核实，采集暂停；可明确启动。";
            Ok(result)
        }
    }
    #[cfg(test)]
    mod tests {
        include!("claude_plan_runtime_tests.rs");
    }
}
#[cfg(target_os = "macos")]
pub(crate) use native::{Runtime, Status};
