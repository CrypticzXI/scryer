use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const APPLICATION_UPGRADE_HELPER_PLAN_SCHEMA: &str = "scryer.upgrade.helper-plan.v1";

/// Durable instructions consumed by the temporary Windows upgrade helper.
///
/// This deliberately lives in the application crate: the executable only needs to
/// perform the small Windows process and file-system operations described here.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ApplicationUpgradeHelperPlan {
    pub schema: String,
    pub mode: ApplicationUpgradeHelperMode,
    pub owner: ApplicationUpgradeHelperOwner,
    pub journal_path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub staged_dir: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub msi_path: Option<PathBuf>,
    pub install_dir: PathBuf,
    #[serde(default)]
    pub replace: Vec<ApplicationUpgradeHelperReplacement>,
    pub backup_suffix: String,
    pub relaunch: ApplicationUpgradeHelperRelaunch,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tray_shutdown_program: Option<PathBuf>,
    pub expected_version: String,
    pub expected_tag: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplicationUpgradeHelperMode {
    PortableZip,
    Msi,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplicationUpgradeHelperOwner {
    Direct,
    Tray,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ApplicationUpgradeHelperReplacement {
    pub from_staged: PathBuf,
    pub to_install: PathBuf,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ApplicationUpgradeHelperRelaunch {
    pub program: PathBuf,
    #[serde(default)]
    pub args: Vec<String>,
    pub cwd: PathBuf,
}

impl ApplicationUpgradeHelperPlan {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != APPLICATION_UPGRADE_HELPER_PLAN_SCHEMA {
            return Err(format!(
                "unsupported upgrade helper plan schema '{}'",
                self.schema
            ));
        }
        if self.journal_path.as_os_str().is_empty()
            || self.install_dir.as_os_str().is_empty()
            || self.relaunch.program.as_os_str().is_empty()
            || self.relaunch.cwd.as_os_str().is_empty()
            || self.expected_version.trim().is_empty()
            || self.expected_tag.trim().is_empty()
        {
            return Err(
                "upgrade helper plan contains an empty required path or version".to_string(),
            );
        }
        if !self.backup_suffix.starts_with(".pre-upgrade-")
            || self.backup_suffix.contains('/')
            || self.backup_suffix.contains('\\')
        {
            return Err("upgrade helper plan has an unsafe backup suffix".to_string());
        }
        if self.owner == ApplicationUpgradeHelperOwner::Tray && self.tray_shutdown_program.is_none()
        {
            return Err(
                "tray-owned upgrade helper plan requires a tray shutdown program".to_string(),
            );
        }

        match self.mode {
            ApplicationUpgradeHelperMode::PortableZip => {
                let staged_dir = self.staged_dir.as_deref().ok_or_else(|| {
                    "portable upgrade helper plan requires staged_dir".to_string()
                })?;
                if self.msi_path.is_some() || self.replace.len() != 2 {
                    return Err(
                        "portable upgrade helper plan requires exactly two replacements and no MSI"
                            .to_string(),
                    );
                }
                let mut names = self
                    .replace
                    .iter()
                    .map(|replacement| {
                        if !replacement.from_staged.starts_with(staged_dir)
                            || replacement.to_install.parent() != Some(self.install_dir.as_path())
                        {
                            return Err(
                                "portable upgrade helper plan has a replacement outside its declared directories"
                                    .to_string(),
                            );
                        }
                        replacement
                            .to_install
                            .file_name()
                            .and_then(|name| name.to_str())
                            .ok_or_else(|| {
                                "portable upgrade helper plan has an invalid replacement name"
                                    .to_string()
                            })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                names.sort_unstable();
                if names != ["scryer-tray.exe", "scryer.exe"] {
                    return Err(
                        "portable upgrade helper plan must replace scryer.exe and scryer-tray.exe"
                            .to_string(),
                    );
                }
            }
            ApplicationUpgradeHelperMode::Msi => {
                if self.staged_dir.is_some() || !self.replace.is_empty() {
                    return Err(
                        "MSI upgrade helper plan must not contain staged replacements".to_string(),
                    );
                }
                if self.msi_path.is_none() {
                    return Err("MSI upgrade helper plan requires msi_path".to_string());
                }
            }
        }
        Ok(())
    }
}

/// The two atomic renames used for a single portable executable replacement.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortableReplacementOperations {
    pub retain_backup_from: PathBuf,
    pub retain_backup_to: PathBuf,
    pub install_from: PathBuf,
    pub install_to: PathBuf,
}

pub fn portable_replacement_operations(
    replacement: &ApplicationUpgradeHelperReplacement,
    backup_suffix: &str,
) -> PortableReplacementOperations {
    PortableReplacementOperations {
        retain_backup_from: replacement.to_install.clone(),
        retain_backup_to: PathBuf::from(format!(
            "{}{}",
            replacement.to_install.display(),
            backup_suffix
        )),
        install_from: replacement.from_staged.clone(),
        install_to: replacement.to_install.clone(),
    }
}

/// Rename operations that undo replacements in reverse order.
///
/// `completed` contains replacements for which the staged member was installed.
/// `backup_only` is the replacement whose original executable was moved aside but
/// whose staged member could not be installed.
pub fn portable_replacement_rollback_operations(
    completed: &[PortableReplacementOperations],
    backup_only: Option<&PortableReplacementOperations>,
) -> Vec<(PathBuf, PathBuf)> {
    let mut rollback = Vec::new();
    if let Some(backup_only) = backup_only {
        rollback.push((
            backup_only.retain_backup_to.clone(),
            backup_only.retain_backup_from.clone(),
        ));
    }
    for operation in completed.iter().rev() {
        rollback.push((operation.install_to.clone(), operation.install_from.clone()));
        rollback.push((
            operation.retain_backup_to.clone(),
            operation.retain_backup_from.clone(),
        ));
    }
    rollback
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MsiHelperJournalTransition {
    Restarting,
    RebootRequired,
    HelperError(String),
}

/// Translate the Windows installer/UAC status into the durable journal state.
pub fn msi_exit_code_transition(code: u32) -> MsiHelperJournalTransition {
    match code {
        0 => MsiHelperJournalTransition::Restarting,
        3010 => MsiHelperJournalTransition::RebootRequired,
        1223 => MsiHelperJournalTransition::HelperError("elevation was declined".to_string()),
        code => MsiHelperJournalTransition::HelperError(format!("installer exit code {code}")),
    }
}

/// Whether a reboot-required journal can be completed on this boot.
pub fn reboot_required_completion_allowed(
    written_at: Option<DateTime<Utc>>,
    boot_time: Option<DateTime<Utc>>,
    expected_version_booted: bool,
    expected_executable_booted: bool,
) -> bool {
    expected_version_booted
        && expected_executable_booted
        && written_at
            .zip(boot_time)
            .is_some_and(|(written_at, boot_time)| boot_time > written_at)
}

pub fn path_is_within(path: &Path, root: &Path) -> bool {
    path.starts_with(root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn portable_plan() -> ApplicationUpgradeHelperPlan {
        ApplicationUpgradeHelperPlan {
            schema: APPLICATION_UPGRADE_HELPER_PLAN_SCHEMA.to_string(),
            mode: ApplicationUpgradeHelperMode::PortableZip,
            owner: ApplicationUpgradeHelperOwner::Tray,
            journal_path: PathBuf::from("C:/data/application-upgrade/journal.json"),
            staged_dir: Some(PathBuf::from(
                "C:/data/application-upgrade/staging/extracted",
            )),
            msi_path: None,
            install_dir: PathBuf::from("C:/Program Files/Scryer"),
            replace: vec![
                ApplicationUpgradeHelperReplacement {
                    from_staged: PathBuf::from(
                        "C:/data/application-upgrade/staging/extracted/scryer.exe",
                    ),
                    to_install: PathBuf::from("C:/Program Files/Scryer/scryer.exe"),
                },
                ApplicationUpgradeHelperReplacement {
                    from_staged: PathBuf::from(
                        "C:/data/application-upgrade/staging/extracted/scryer-tray.exe",
                    ),
                    to_install: PathBuf::from("C:/Program Files/Scryer/scryer-tray.exe"),
                },
            ],
            backup_suffix: ".pre-upgrade-1.2.3".to_string(),
            relaunch: ApplicationUpgradeHelperRelaunch {
                program: PathBuf::from("C:/Program Files/Scryer/scryer-tray.exe"),
                args: vec!["--login-start".to_string()],
                cwd: PathBuf::from("C:/Program Files/Scryer"),
            },
            tray_shutdown_program: Some(PathBuf::from("C:/Program Files/Scryer/scryer-tray.exe")),
            expected_version: "1.2.4".to_string(),
            expected_tag: "v1.2.4".to_string(),
        }
    }

    #[test]
    fn helper_plan_round_trips_and_validates() {
        let plan = portable_plan();
        let decoded: ApplicationUpgradeHelperPlan =
            serde_json::from_slice(&serde_json::to_vec(&plan).expect("encode plan"))
                .expect("decode plan");
        assert_eq!(decoded, plan);
        decoded.validate().expect("valid plan");
    }

    #[test]
    fn helper_plan_rejects_invalid_mode_specific_fields() {
        let mut plan = portable_plan();
        plan.replace.pop();
        assert!(plan.validate().is_err());

        let mut plan = portable_plan();
        plan.backup_suffix = "../escape".to_string();
        assert!(plan.validate().is_err());
    }

    #[test]
    fn msi_helper_plan_requires_only_the_installer_path() {
        let mut plan = portable_plan();
        plan.mode = ApplicationUpgradeHelperMode::Msi;
        plan.owner = ApplicationUpgradeHelperOwner::Direct;
        plan.staged_dir = None;
        plan.msi_path = Some(PathBuf::from(
            "C:/data/application-upgrade/staging/artifact",
        ));
        plan.replace.clear();
        plan.tray_shutdown_program = None;
        plan.relaunch = ApplicationUpgradeHelperRelaunch {
            program: PathBuf::from("C:/Program Files/Scryer/scryer.exe"),
            args: Vec::new(),
            cwd: PathBuf::from("C:/Program Files/Scryer"),
        };
        plan.validate().expect("valid MSI plan");
        let value = serde_json::to_value(plan).expect("encode MSI plan");
        assert!(value.get("staged_dir").is_none());
        assert!(value.get("msi_path").is_some());
    }

    #[test]
    fn msi_exit_codes_map_to_durable_transitions() {
        assert_eq!(
            msi_exit_code_transition(0),
            MsiHelperJournalTransition::Restarting
        );
        assert_eq!(
            msi_exit_code_transition(3010),
            MsiHelperJournalTransition::RebootRequired
        );
        assert_eq!(
            msi_exit_code_transition(1223),
            MsiHelperJournalTransition::HelperError("elevation was declined".to_string())
        );
        assert_eq!(
            msi_exit_code_transition(1603),
            MsiHelperJournalTransition::HelperError("installer exit code 1603".to_string())
        );
    }

    #[test]
    fn replacement_rollback_reverses_completed_members_before_restoring_backups() {
        let plan = portable_plan();
        let operations = plan
            .replace
            .iter()
            .map(|replacement| portable_replacement_operations(replacement, &plan.backup_suffix))
            .collect::<Vec<_>>();
        assert_eq!(
            portable_replacement_rollback_operations(&operations, None),
            vec![
                (
                    operations[1].install_to.clone(),
                    operations[1].install_from.clone()
                ),
                (
                    operations[1].retain_backup_to.clone(),
                    operations[1].retain_backup_from.clone()
                ),
                (
                    operations[0].install_to.clone(),
                    operations[0].install_from.clone()
                ),
                (
                    operations[0].retain_backup_to.clone(),
                    operations[0].retain_backup_from.clone()
                ),
            ]
        );
        assert_eq!(
            portable_replacement_rollback_operations(&operations[..1], Some(&operations[1]))[0],
            (
                operations[1].retain_backup_to.clone(),
                operations[1].retain_backup_from.clone()
            )
        );
    }

    #[test]
    fn reboot_completion_requires_a_boot_after_the_journal_write() {
        let written_at = Utc.with_ymd_and_hms(2026, 8, 24, 12, 0, 0).single();
        let rebooted = Utc.with_ymd_and_hms(2026, 8, 24, 12, 1, 0).single();
        let not_rebooted = Utc.with_ymd_and_hms(2026, 8, 24, 11, 59, 0).single();
        assert!(reboot_required_completion_allowed(
            written_at, rebooted, true, true
        ));
        assert!(!reboot_required_completion_allowed(
            written_at,
            not_rebooted,
            true,
            true
        ));
        assert!(!reboot_required_completion_allowed(
            None, rebooted, true, true
        ));
        assert!(!reboot_required_completion_allowed(
            written_at, rebooted, false, true
        ));
    }
}
