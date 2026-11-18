#[cfg(target_os = "macos")]
use keyring::{Entry, Error as KeyringError};
#[cfg(target_os = "macos")]
use std::fs::{self, OpenOptions};
#[cfg(target_os = "macos")]
use std::io::{self, Write};
#[cfg(target_os = "macos")]
use std::path::{Path, PathBuf};

#[cfg(target_os = "macos")]
fn main() {
    let mut args = std::env::args().skip(1);
    let run_id = args.next().unwrap_or_default();
    let Some(result_path) = args.next().map(PathBuf::from) else {
        eprintln!("native-policy-anchor-probe: RESULT_PATH_MISSING");
        std::process::exit(2);
    };
    if args.next().is_some() || !valid_result_path(&result_path) {
        eprintln!("native-policy-anchor-probe: RESULT_PATH_INVALID");
        std::process::exit(2);
    }

    let outcome = run(&run_id);
    if publish_result(&result_path, &outcome).is_err() {
        eprintln!("native-policy-anchor-probe: RESULT_WRITE_FAILED");
        std::process::exit(2);
    }
    match outcome {
        Ok(()) => println!("native-policy-anchor-probe: create/read/update/delete passed"),
        Err(code) => {
            eprintln!("native-policy-anchor-probe: {code}");
            std::process::exit(2);
        }
    }
}

#[cfg(target_os = "macos")]
fn run(run_id: &str) -> Result<(), String> {
    if run_id.len() != 32 || !run_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("RUN_ID_INVALID".to_owned());
    }

    // The one-use account is unique to this isolated native acceptance run.
    // Refuse any pre-existing item so cleanup can never delete user data.
    let account = format!("native-acceptance-{run_id}");
    let entry = Entry::new("dev.agentisland.policy-anchor-acceptance", &account)
        .map_err(|error| keyring_error_code("ENTRY_CREATE", &error))?;
    match entry.get_password() {
        Err(KeyringError::NoEntry) => {}
        Ok(_) => return Err("UNEXPECTED_PREEXISTING_ENTRY".to_owned()),
        Err(error) => return Err(keyring_error_code("ABSENT_CHECK", &error)),
    }

    let first = format!("anchor-write-{run_id}");
    let second = format!("anchor-update-{run_id}");
    let mut item_written = false;
    let result = (|| {
        // The account was observed absent above; set this before the native
        // call so even an ambiguous failure receives best-effort cleanup.
        item_written = true;
        entry
            .set_password(&first)
            .map_err(|error| keyring_error_code("CREATE", &error))?;
        if entry
            .get_password()
            .map_err(|error| keyring_error_code("READ_BACK", &error))?
            != first
        {
            return Err("READ_BACK_MISMATCH".to_owned());
        }
        entry
            .set_password(&second)
            .map_err(|error| keyring_error_code("UPDATE", &error))?;
        if entry
            .get_password()
            .map_err(|error| keyring_error_code("UPDATED_READ", &error))?
            != second
        {
            return Err("UPDATED_READ_MISMATCH".to_owned());
        }
        entry
            .delete_credential()
            .map_err(|error| keyring_error_code("DELETE", &error))?;
        if !matches!(entry.get_password(), Err(KeyringError::NoEntry)) {
            return Err("DELETE_VERIFICATION_FAILED".to_owned());
        }
        item_written = false;
        Ok(())
    })();

    if item_written {
        match entry.delete_credential() {
            Ok(()) | Err(KeyringError::NoEntry) => {}
            Err(error) => {
                let cleanup = keyring_error_code("CLEANUP", &error);
                return Err(match result {
                    Ok(()) => cleanup,
                    Err(original) => format!("{cleanup}; ORIGINAL_{original}"),
                });
            }
        }
    }
    result
}

#[cfg(target_os = "macos")]
fn keyring_error_code(stage: &str, error: &KeyringError) -> String {
    let status = match error {
        KeyringError::PlatformFailure(source) | KeyringError::NoStorageAccess(source) => source
            .as_ref()
            .downcast_ref::<security_framework::base::Error>()
            .map(|error| error.code()),
        _ => None,
    };
    match status {
        Some(status) => format!("{stage}_OSSTATUS_{status}"),
        None => format!("{stage}_KEYRING_ERROR"),
    }
}

#[cfg(target_os = "macos")]
fn valid_result_path(path: &Path) -> bool {
    if !path.is_absolute()
        || path
            .file_name()
            .is_none_or(|name| name != "native-policy-anchor-result.json")
    {
        return false;
    }
    let Some(parent) = path.parent() else {
        return false;
    };
    if parent.file_name().is_none_or(|name| name != ".build") {
        return false;
    }
    matches!(fs::symlink_metadata(parent), Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink())
        && matches!(fs::symlink_metadata(path), Err(error) if error.kind() == io::ErrorKind::NotFound)
}

#[cfg(target_os = "macos")]
fn publish_result(path: &Path, outcome: &Result<(), String>) -> io::Result<()> {
    let bytes = match outcome {
        Ok(()) => br#"{"result":"passed"}"#.to_vec(),
        Err(code) => format!(r#"{{"result":"failed","error":"{code}"}}"#).into_bytes(),
    };
    let staged = path.with_extension("pending");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staged)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(staged, path)
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("native-policy-anchor-probe: unsupported platform");
    std::process::exit(2);
}
