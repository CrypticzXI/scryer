use std::path::Path;

#[cfg(windows)]
use scryer_application::application_upgrade::{
    ApplicationUpgradeHelperMode, ApplicationUpgradeHelperOwner,
};
use scryer_application::application_upgrade::{
    ApplicationUpgradeHelperPlan, MsiHelperJournalTransition, msi_exit_code_transition,
};

pub fn maybe_run_upgrade_helper() -> Result<bool, String> {
    let mut args = std::env::args_os();
    let _program = args.next();
    if args.next().as_deref() != Some(std::ffi::OsStr::new("--upgrade-helper")) {
        return Ok(false);
    }
    let plan_path = args
        .next()
        .ok_or_else(|| "--upgrade-helper requires a plan path".to_string())?;
    if args.next().is_some() {
        return Err("--upgrade-helper accepts exactly one plan path".to_string());
    }
    let plan = read_plan(Path::new(&plan_path))?;
    #[cfg(windows)]
    {
        run_windows_helper(&plan)?;
        Ok(true)
    }
    #[cfg(not(windows))]
    {
        let _ = plan;
        Err("--upgrade-helper is only available on Windows".to_string())
    }
}

fn read_plan(path: &Path) -> Result<ApplicationUpgradeHelperPlan, String> {
    let bytes = std::fs::read(path)
        .map_err(|error| format!("failed to read application upgrade helper plan: {error}"))?;
    let plan: ApplicationUpgradeHelperPlan = serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid application upgrade helper plan: {error}"))?;
    plan.validate()?;
    Ok(plan)
}

#[cfg_attr(not(windows), allow(dead_code))]
fn msi_launch_error_transition(win32_error: u32) -> MsiHelperJournalTransition {
    if win32_error == 1223 {
        msi_exit_code_transition(win32_error)
    } else {
        MsiHelperJournalTransition::HelperError(format!(
            "failed to launch elevated installer (win32 error {win32_error})"
        ))
    }
}

#[cfg(windows)]
fn run_windows_helper(plan: &ApplicationUpgradeHelperPlan) -> Result<(), String> {
    let outcome = (|| {
        stop_owner(plan)?;
        wait_for_file_release(&installed_executables(plan))?;
        match plan.mode {
            ApplicationUpgradeHelperMode::PortableZip => apply_portable_replacements(plan),
            ApplicationUpgradeHelperMode::Msi => run_msi_installer(plan),
        }
    })();

    if let Err(error) = &outcome {
        write_helper_error(plan, error.clone());
    }
    if let Err(error) = relaunch_owner(plan) {
        let message = format!("failed to relaunch application after upgrade helper: {error}");
        write_helper_error(plan, message.clone());
        return Err(message);
    }
    outcome
}

#[cfg(windows)]
fn stop_owner(plan: &ApplicationUpgradeHelperPlan) -> Result<(), String> {
    if plan.owner != ApplicationUpgradeHelperOwner::Tray {
        return Ok(());
    }
    let program = plan
        .tray_shutdown_program
        .as_ref()
        .ok_or_else(|| "tray-owned helper plan has no shutdown program".to_string())?;
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

    let status = std::process::Command::new(program)
        .arg("--shutdown")
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map_err(|error| format!("failed to invoke tray shutdown: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("tray shutdown exited with {status}"))
    }
}

#[cfg(windows)]
fn installed_executables(plan: &ApplicationUpgradeHelperPlan) -> Vec<std::path::PathBuf> {
    match plan.mode {
        ApplicationUpgradeHelperMode::PortableZip => plan
            .replace
            .iter()
            .map(|replacement| replacement.to_install.clone())
            .collect(),
        ApplicationUpgradeHelperMode::Msi => vec![
            plan.install_dir.join("scryer.exe"),
            plan.install_dir.join("scryer-tray.exe"),
        ],
    }
}

#[cfg(windows)]
fn wait_for_file_release(paths: &[std::path::PathBuf]) -> Result<(), String> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        let handles = paths
            .iter()
            .map(|path| {
                std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(path)
            })
            .collect::<Result<Vec<_>, _>>();
        if handles.is_ok() {
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            return Err("timed out waiting for installed executables to be released".to_string());
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
}

#[cfg(windows)]
fn apply_portable_replacements(plan: &ApplicationUpgradeHelperPlan) -> Result<(), String> {
    use scryer_application::application_upgrade::{
        portable_replacement_operations, portable_replacement_rollback_operations,
    };

    let mut completed = Vec::new();
    for replacement in &plan.replace {
        let operations = portable_replacement_operations(replacement, &plan.backup_suffix);
        if let Err(error) =
            std::fs::rename(&operations.retain_backup_from, &operations.retain_backup_to)
        {
            rollback_replacements(&completed, None)?;
            return Err(format!(
                "failed to retain installed executable backup: {error}"
            ));
        }
        if let Err(error) = std::fs::rename(&operations.install_from, &operations.install_to) {
            rollback_replacements(&completed, Some(&operations))?;
            return Err(format!("failed to install staged executable: {error}"));
        }
        completed.push(operations);
    }
    Ok(())
}

#[cfg(windows)]
fn rollback_replacements(
    completed: &[scryer_application::application_upgrade::PortableReplacementOperations],
    backup_only: Option<&scryer_application::application_upgrade::PortableReplacementOperations>,
) -> Result<(), String> {
    use scryer_application::application_upgrade::portable_replacement_rollback_operations;

    let mut errors = Vec::new();
    for (from, to) in portable_replacement_rollback_operations(completed, backup_only) {
        if let Err(error) = std::fs::rename(&from, &to) {
            errors.push(format!("{} -> {}: {error}", from.display(), to.display()));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!("rollback failed: {}", errors.join("; ")))
    }
}

#[cfg(windows)]
fn run_msi_installer(plan: &ApplicationUpgradeHelperPlan) -> Result<(), String> {
    use core::mem::size_of;
    use std::ptr;
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, INFINITE, WaitForSingleObject,
    };
    use windows_sys::Win32::UI::Shell::{
        SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, SHELLEXECUTEINFOW_0, ShellExecuteExW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE;

    let msi_path = plan
        .msi_path
        .as_ref()
        .ok_or_else(|| "MSI helper plan has no installer path".to_string())?;
    let verb = wide("runas");
    let program = wide("msiexec.exe");
    let parameters = wide(&format!(
        "/i \"{}\" /passive /norestart",
        msi_path.display()
    ));
    // SAFETY: Zero is the documented initializer for this Win32 structure; all
    // pointer fields below refer to NUL-terminated buffers that outlive the call.
    let mut execute: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    execute.cbSize = size_of::<SHELLEXECUTEINFOW>() as u32;
    execute.fMask = SEE_MASK_NOCLOSEPROCESS;
    execute.lpVerb = verb.as_ptr();
    execute.lpFile = program.as_ptr();
    execute.lpParameters = parameters.as_ptr();
    execute.lpDirectory = ptr::null();
    execute.nShow = SW_HIDE;
    execute.Anonymous = SHELLEXECUTEINFOW_0 { hIcon: 0 };
    // SAFETY: `execute` is fully initialized for ShellExecuteExW and remains valid
    // through the call. The returned process handle is closed below.
    if unsafe { ShellExecuteExW(&mut execute) } == 0 {
        // SAFETY: GetLastError reads the thread-local error set by ShellExecuteExW.
        let transition = msi_launch_error_transition(unsafe { GetLastError() });
        return write_msi_transition(plan, transition);
    }
    // SAFETY: ShellExecuteExW returned a process handle because SEE_MASK_NOCLOSEPROCESS was set.
    unsafe { WaitForSingleObject(execute.hProcess, INFINITE) };
    let mut exit_code = 0_u32;
    // SAFETY: The process handle is valid until CloseHandle below and exit_code is writable.
    let exit_status = unsafe { GetExitCodeProcess(execute.hProcess, &mut exit_code) };
    // SAFETY: This helper owns the process handle returned by ShellExecuteExW.
    unsafe { CloseHandle(execute.hProcess) };
    if exit_status == 0 {
        return Err("failed to read MSI installer exit code".to_string());
    }
    write_msi_transition(plan, msi_exit_code_transition(exit_code))
}

#[cfg(windows)]
fn write_msi_transition(
    plan: &ApplicationUpgradeHelperPlan,
    transition: scryer_application::application_upgrade::MsiHelperJournalTransition,
) -> Result<(), String> {
    use scryer_application::application_upgrade::{
        application_upgrade_helper_update_journal, phases,
    };

    let (phase, error) = match transition {
        scryer_application::application_upgrade::MsiHelperJournalTransition::Restarting => {
            (phases::RESTARTING, None)
        }
        scryer_application::application_upgrade::MsiHelperJournalTransition::RebootRequired => {
            (phases::REBOOT_REQUIRED, None)
        }
        scryer_application::application_upgrade::MsiHelperJournalTransition::HelperError(error) => {
            (phases::RESTARTING, Some(error))
        }
    };
    application_upgrade_helper_update_journal(&plan.journal_path, phase, error)
        .map_err(|error| format!("failed to update application upgrade journal: {error}"))
}

#[cfg(windows)]
fn write_helper_error(plan: &ApplicationUpgradeHelperPlan, error: String) {
    use scryer_application::application_upgrade::{
        application_upgrade_helper_update_journal, phases,
    };

    if let Err(write_error) = application_upgrade_helper_update_journal(
        &plan.journal_path,
        phases::RESTARTING,
        Some(error),
    ) {
        eprintln!("failed to record application upgrade helper error: {write_error}");
    }
}

#[cfg(windows)]
fn relaunch_owner(plan: &ApplicationUpgradeHelperPlan) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

    std::process::Command::new(&plan.relaunch.program)
        .args(&plan.relaunch.args)
        .current_dir(&plan.relaunch.cwd)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("failed to relaunch application owner: {error}"))
}

#[cfg(windows)]
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elevated_installer_launch_errors_are_not_reported_as_installer_exit_codes() {
        assert_eq!(
            msi_launch_error_transition(1223),
            MsiHelperJournalTransition::HelperError("elevation was declined".to_string())
        );
        assert_eq!(
            msi_launch_error_transition(2),
            MsiHelperJournalTransition::HelperError(
                "failed to launch elevated installer (win32 error 2)".to_string()
            )
        );
    }
}
