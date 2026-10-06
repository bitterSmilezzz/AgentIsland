//! A user-invoked native directory picker; never called by background collection.
#[cfg(target_os = "macos")]
pub(crate) fn choose() -> Result<Option<std::path::PathBuf>, String> {
    use objc2::{
        class, msg_send,
        rc::Retained,
        runtime::{AnyObject, Bool},
    };
    use objc2_foundation::NSString;
    if objc2::MainThreadMarker::new().is_none() {
        return Err("目录选择必须在主线程执行".into());
    }
    unsafe {
        let panel: Retained<AnyObject> = msg_send![class!(NSOpenPanel), openPanel];
        let _: () = msg_send![&*panel,setCanChooseDirectories:Bool::YES];
        let _: () = msg_send![&*panel,setCanChooseFiles:Bool::NO];
        let _: () = msg_send![&*panel,setAllowsMultipleSelection:Bool::NO];
        let _: () = msg_send![&*panel,setCanCreateDirectories:Bool::NO];
        let _: () = msg_send![&*panel,setCanDownloadUbiquitousContents:Bool::NO];
        let _: () = msg_send![&*panel,setTitle:&*NSString::from_str("选择技能目录")];
        let _: () = msg_send![&*panel,setMessage:&*NSString::from_str("选择根部包含 SKILL.md 的本地目录。下一步先预览，不立即安装。")];
        let result: isize = msg_send![&*panel, runModal];
        if result != 1 {
            return Ok(None);
        }
        let url: Option<Retained<AnyObject>> = msg_send![&*panel, URL];
        let url = url.ok_or("没有取得所选目录")?;
        let local: Bool = msg_send![&*url, isFileURL];
        if !local.as_bool() {
            return Err("请选择本地技能目录".into());
        }
        let path: Option<Retained<NSString>> = msg_send![&*url, path];
        Ok(Some(std::path::PathBuf::from(
            path.ok_or("目录路径不可读")?.to_string(),
        )))
    }
}
