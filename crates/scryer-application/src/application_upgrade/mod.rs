mod engine;
mod helper_plan;
mod installation;
pub mod manifest;
mod restart;

pub use engine::{
    ApplicationUpgradeJobAccepted, ApplicationUpgradeJobRequest, ApplicationUpgradeJournal,
    ApplicationUpgradeProgress, application_upgrade_helper_update_journal, phases,
};
pub use helper_plan::{
    APPLICATION_UPGRADE_HELPER_PLAN_SCHEMA, ApplicationUpgradeHelperMode,
    ApplicationUpgradeHelperOwner, ApplicationUpgradeHelperPlan, ApplicationUpgradeHelperRelaunch,
    ApplicationUpgradeHelperReplacement, MsiHelperJournalTransition, PortableReplacementOperations,
    msi_exit_code_transition, path_is_within, portable_replacement_operations,
    portable_replacement_rollback_operations, reboot_required_completion_allowed,
};
pub use installation::{
    EligibilityReason, InstallationAssessment, InstallationEvidence, InstallationKind,
    InstallationOs, ManagementOwner, classify_installation,
};
pub use restart::ApplicationUpgradeRestartHandle;
