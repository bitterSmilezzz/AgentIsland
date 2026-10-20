//! All pre-GUI entrypoints share the same default-disabled file logger.
//! Adding a handler never requires a second registration in the logger.
pub(crate) type Handler = fn(&[String]) -> Option<i32>;

fn dispatch_with(argv: &[String], handlers: &[Handler]) -> Option<i32> {
    handlers.iter().find_map(|handler| handler(argv))
}

pub(crate) fn dispatch(argv: &[String]) -> Option<i32> {
    dispatch_with(argv, &[
        crate::notify_ledger_probe::cli,
        crate::gateway_auth_probe::cli,
        crate::remote_delivery_probe::cli,
        crate::claude_plan_receiver::cli,
        crate::resource_diagnostics::cli,
        crate::cli::try_run,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_probe(argv: &[String]) -> Option<i32> {
        if argv.get(1).map(String::as_str) != Some("--fresh-probe") { return None; }
        crate::log_line("synthetic new probe diagnostic");
        std::thread::spawn(|| crate::log_line("synthetic new probe worker")).join().unwrap();
        Some(if argv.len() == 2 { 0 } else { 2 })
    }

    #[test]
    fn newly_registered_handlers_need_no_logger_registration() {
        assert_eq!(dispatch_with(&["agentisland".into(), "--fresh-probe".into()], &[fresh_probe]), Some(0));
        assert_eq!(dispatch_with(&["agentisland".into(), "--fresh-probe".into(), "invalid".into()], &[fresh_probe]), Some(2));
        assert_eq!(dispatch_with(&["agentisland".into(), "--demo".into()], &[fresh_probe]), None);
    }

    #[test]
    fn first_handled_result_wins_and_none_continues() {
        fn decline(_: &[String]) -> Option<i32> { None }
        fn fail(_: &[String]) -> Option<i32> { Some(2) }
        fn never(_: &[String]) -> Option<i32> { panic!("a handled command must not fall through"); }
        assert_eq!(dispatch_with(&[], &[decline, fail, never]), Some(2));
    }

    #[test]
    fn actual_probe_argument_errors_do_not_fall_through_to_gui() {
        for flag in ["--notify-ledger-probe", "--gateway-auth-probe", "--remote-delivery-probe"] {
            let argv = ["agentisland".into(), flag.into(), "invalid".into()];
            assert_eq!(dispatch(&argv), Some(2), "{flag}");
        }
        let argv = ["agentisland".into(), "--memory-core-probe".into(), "--probe-samples".into(), "0".into()];
        assert_eq!(dispatch(&argv), Some(2));
        assert_eq!(dispatch(&["agentisland".into(), "--claude-plan-hook".into(), "invalid".into()]), Some(1));
        assert_eq!(dispatch(&["agentisland".into(), "unknown-command".into()]), Some(2));
        assert_eq!(dispatch(&["agentisland".into(), "--demo".into()]), None);
    }
}
