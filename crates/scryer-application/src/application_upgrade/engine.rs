use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

use flate2::read::GzDecoder;
use futures_util::StreamExt;
use semver::Version;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

use crate::application_upgrade::InstallationKind;
use crate::application_upgrade::manifest::{
    UPGRADE_MANIFEST_MAX_BYTES, UpgradeArchitecture, UpgradeArchive, UpgradeArtifact,
    UpgradeChannel, UpgradeManifest, UpgradePlatform, parse_and_validate_upgrade_manifest,
    scryer_release_required_signer,
};
use crate::domain_events::DomainEventActor;
use crate::plugins::catalog::verify_signed_blob;
use crate::{
    AppError, AppResult, AppUseCase, JobKey, JobRun, JobRunRecord, JobRunStatus, JobTriggerSource,
    SCRYER_VERSION, filesystem_space_raw,
};
use scryer_domain::{
    DomainEventPayload, Id, JobRunCompletedEventData, JobRunFailedEventData,
    JobRunStartedEventData, User,
};

/// Stable progress phase names consumed by the application-upgrade UI.
pub mod phases {
    pub const CHECKING: &str = "checking";
    pub const DOWNLOADING: &str = "downloading";
    pub const VERIFYING: &str = "verifying";
    pub const STAGING: &str = "staging";
    pub const APPLYING: &str = "applying";
    pub const AWAITING_ELEVATION: &str = "awaiting_elevation";
    pub const RESTARTING: &str = "restarting";
    pub const REBOOT_REQUIRED: &str = "reboot_required";
}

const UPGRADE_BUNDLE_MAX_BYTES: u64 = 256 * 1024;
const UPGRADE_STAGING_RESERVE_BYTES: u64 = 64 * 1024 * 1024;
const DOWNLOAD_PROGRESS_INTERVAL: Duration = Duration::from_millis(500);
const JOURNAL_SCHEMA: &str = "scryer.upgrade.journal.v1";
const WINDOWS_APPLY_UNSUPPORTED: &str = "windows apply is not yet supported";

/// Progress persisted in `workflow_operations.progress_json` for an application upgrade.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationUpgradeProgress {
    pub status: String,
    pub phase: String,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub target_version: String,
    pub target_tag: String,
    pub error: Option<String>,
}

/// Internal start request assembled by the GraphQL mutation after installation assessment.
#[derive(Clone, Debug)]
pub struct ApplicationUpgradeJobRequest {
    pub expected_tag: String,
    pub expected_version: String,
    pub installation_kind: InstallationKind,
    /// Tests and nonstandard executable hosts may provide the startup evidence path directly.
    pub executable_path: Option<PathBuf>,
}

/// Accepted durable job run returned once the asynchronous engine is registered.
#[derive(Clone, Debug)]
pub struct ApplicationUpgradeJobAccepted {
    pub job_run: JobRun,
}

/// Crash-safe handoff between applying an upgrade and validating the next boot.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ApplicationUpgradeJournal {
    pub schema: String,
    pub run_id: String,
    pub expected_version: String,
    pub expected_tag: String,
    pub executable_path: PathBuf,
    pub backup_path: PathBuf,
    pub phase: String,
    pub helper_error: Option<String>,
}

#[derive(Clone, Debug)]
struct AppliedPortableUpgrade {
    executable_path: PathBuf,
    backup_path: PathBuf,
}

type UpgradeSpaceCheck = fn(&Path, u64) -> AppResult<()>;
type UpgradeRename = fn(&Path, &Path) -> std::io::Result<()>;

fn rename_path(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::rename(from, to)
}

struct UpgradePipelineDependencies<'a> {
    client: &'a reqwest::Client,
    artifact_url_override: Option<&'a str>,
    ensure_available_space: UpgradeSpaceCheck,
    rename: UpgradeRename,
}

impl ApplicationUpgradeProgress {
    fn checking(request: &ApplicationUpgradeJobRequest) -> Self {
        Self {
            status: JobRunStatus::Running.as_str().to_string(),
            phase: phases::CHECKING.to_string(),
            downloaded_bytes: 0,
            total_bytes: 0,
            target_version: request.expected_version.clone(),
            target_tag: request.expected_tag.clone(),
            error: None,
        }
    }
}

impl AppUseCase {
    /// Validate the signed-update notice and begin the single-flight application-upgrade job.
    pub async fn start_application_upgrade_job(
        &self,
        actor: &User,
        request: ApplicationUpgradeJobRequest,
    ) -> AppResult<ApplicationUpgradeJobAccepted> {
        if request.expected_tag.trim().is_empty() {
            return Err(AppError::Validation(
                "expectedTag must not be empty".to_string(),
            ));
        }
        if request.expected_version.trim().is_empty() {
            return Err(AppError::Validation(
                "expectedVersion must not be empty".to_string(),
            ));
        }

        let notice = self.smg_scryer_update_notice().await?.ok_or_else(|| {
            AppError::Validation("no application update notice is available".to_string())
        })?;
        if !notice.available {
            return Err(AppError::Validation(
                "the application update notice is not available".to_string(),
            ));
        }
        if notice.latest_tag != request.expected_tag {
            return Err(AppError::Validation(
                "expectedTag does not match the current application update notice".to_string(),
            ));
        }
        if notice.latest_version != request.expected_version {
            return Err(AppError::Validation(
                "expectedVersion does not match the current application update notice".to_string(),
            ));
        }

        let expected_version = Version::parse(&request.expected_version).map_err(|error| {
            AppError::Validation(format!("expectedVersion must be valid semver: {error}"))
        })?;
        let running_version = Version::parse(SCRYER_VERSION).map_err(|error| {
            AppError::Repository(format!("running application version is invalid: {error}"))
        })?;
        if expected_version <= running_version {
            return Err(AppError::Validation(
                "expectedVersion must be strictly newer than the running version".to_string(),
            ));
        }

        if !matches!(
            request.installation_kind,
            InstallationKind::Portable | InstallationKind::DirectMsi
        ) {
            return Err(AppError::Validation(
                "application upgrade installation is not eligible".to_string(),
            ));
        }

        let maintenance_guard = self.try_acquire_system_maintenance()?;
        if self
            .runtime
            .jobs
            .job_run_tracker
            .has_active_job(JobKey::ApplicationUpgrade)
            .await
        {
            return Err(AppError::Validation(
                "an application upgrade job is already running".to_string(),
            ));
        }

        let now = chrono::Utc::now();
        let mut run = JobRunRecord {
            id: Id::new().0,
            job_key: JobKey::ApplicationUpgrade,
            operation_type: format!(
                "application_upgrade:{SCRYER_VERSION}->{}",
                request.expected_version
            ),
            status: JobRunStatus::Running,
            trigger_source: JobTriggerSource::Manual,
            actor_user_id: Some(actor.id.clone()),
            progress_json: serde_json::to_string(&ApplicationUpgradeProgress::checking(&request))
                .ok(),
            summary_json: None,
            summary_text: None,
            error_text: None,
            started_at: now,
            completed_at: None,
            created_at: now,
            updated_at: now,
        };
        run = self.services.events.job_runs.create_job_run(&run).await?;
        let job_run = JobRun::from_record(&run, None);
        self.runtime
            .jobs
            .job_run_tracker
            .upsert_active_run(job_run.clone())
            .await;

        let actor_event = DomainEventActor::from(actor);
        let _ = self
            .append_domain_event(crate::domain_events::new_job_run_domain_event(
                actor_event.clone(),
                run.id.clone(),
                DomainEventPayload::JobRunStarted(JobRunStartedEventData {
                    run_id: run.id.clone(),
                    job_key: run.job_key.as_str().to_string(),
                    operation_type: run.operation_type.clone(),
                    trigger_source: run.trigger_source.as_str().to_string(),
                }),
            ))
            .await;

        let app = self.clone();
        tokio::spawn(async move {
            app.run_application_upgrade_job(run, actor_event, request, maintenance_guard)
                .await;
        });

        Ok(ApplicationUpgradeJobAccepted { job_run })
    }

    /// Return the current tracked run and the newest persisted run for the upgrade status query.
    pub async fn application_upgrade_job_runs(
        &self,
    ) -> AppResult<(Option<JobRun>, Option<JobRun>)> {
        let active = self
            .runtime
            .jobs
            .job_run_tracker
            .active_run_for_job(JobKey::ApplicationUpgrade)
            .await;
        let latest = self
            .services
            .events
            .job_runs
            .list_job_runs(Some(JobKey::ApplicationUpgrade), 1)
            .await?
            .into_iter()
            .next()
            .map(|record| JobRun::from_record(&record, None));
        Ok((active, latest))
    }

    async fn run_application_upgrade_job(
        &self,
        mut run: JobRunRecord,
        actor: DomainEventActor,
        request: ApplicationUpgradeJobRequest,
        _maintenance_guard: tokio::sync::OwnedMutexGuard<()>,
    ) {
        let result = self.execute_application_upgrade(&mut run, &request).await;
        if let Err(error) = result {
            self.cleanup_application_upgrade_staging();
            if let Err(finish_error) = self
                .finish_application_upgrade_failure(&mut run, actor, error.to_string())
                .await
            {
                tracing::error!(error = %finish_error, run_id = %run.id, "failed to finish application upgrade job");
            }
        }
    }

    async fn execute_application_upgrade(
        &self,
        run: &mut JobRunRecord,
        request: &ApplicationUpgradeJobRequest,
    ) -> AppResult<()> {
        self.update_application_upgrade_progress(
            run,
            ApplicationUpgradeProgress::checking(request),
        )
        .await?;
        let client = application_upgrade_http_client()?;
        let manifest_url =
            release_asset_url(&request.expected_tag, "scryer-upgrade-manifest.json")?;
        let bundle_url = release_asset_url(
            &request.expected_tag,
            "scryer-upgrade-manifest.json.sigstore.json",
        )?;
        let manifest_raw = fetch_capped_bytes(
            &client,
            manifest_url.as_str(),
            UPGRADE_MANIFEST_MAX_BYTES,
            "upgrade manifest",
        )
        .await?;
        let bundle_raw = fetch_capped_bytes(
            &client,
            bundle_url.as_str(),
            UPGRADE_BUNDLE_MAX_BYTES,
            "upgrade manifest signature bundle",
        )
        .await?;
        verify_upgrade_manifest_signature(manifest_raw.clone(), bundle_raw).await?;
        let manifest = parse_and_validate_upgrade_manifest(&manifest_raw)?;
        self.run_upgrade_pipeline(run, request, &manifest, &client, None)
            .await
    }

    async fn run_upgrade_pipeline(
        &self,
        run: &mut JobRunRecord,
        request: &ApplicationUpgradeJobRequest,
        manifest: &UpgradeManifest,
        client: &reqwest::Client,
        artifact_url_override: Option<&str>,
    ) -> AppResult<()> {
        self.run_upgrade_pipeline_with_dependencies(
            run,
            request,
            manifest,
            UpgradePipelineDependencies {
                client,
                artifact_url_override,
                ensure_available_space,
                rename: rename_path,
            },
        )
        .await
    }

    async fn run_upgrade_pipeline_with_dependencies(
        &self,
        run: &mut JobRunRecord,
        request: &ApplicationUpgradeJobRequest,
        manifest: &UpgradeManifest,
        dependencies: UpgradePipelineDependencies<'_>,
    ) -> AppResult<()> {
        if manifest.tag != request.expected_tag {
            return Err(AppError::Validation(
                "upgrade manifest tag does not match expectedTag".to_string(),
            ));
        }
        if manifest.version != request.expected_version {
            return Err(AppError::Validation(
                "upgrade manifest version does not match expectedVersion".to_string(),
            ));
        }
        let artifact = select_artifact(manifest, request.installation_kind)?.clone();

        self.update_application_upgrade_progress(
            run,
            ApplicationUpgradeProgress {
                phase: phases::DOWNLOADING.to_string(),
                total_bytes: artifact.size,
                ..ApplicationUpgradeProgress::checking(request)
            },
        )
        .await?;
        let staging_dir = self.application_upgrade_staging_dir();
        recreate_staging_dir(&staging_dir)?;
        (dependencies.ensure_available_space)(
            &staging_dir,
            artifact.size.saturating_add(UPGRADE_STAGING_RESERVE_BYTES),
        )?;
        let download_path = staging_dir.join("artifact");
        download_artifact(
            self,
            run,
            request,
            dependencies.client,
            &artifact,
            dependencies.artifact_url_override,
            &download_path,
        )
        .await?;

        self.update_application_upgrade_progress(
            run,
            ApplicationUpgradeProgress {
                phase: phases::VERIFYING.to_string(),
                downloaded_bytes: artifact.size,
                total_bytes: artifact.size,
                ..ApplicationUpgradeProgress::checking(request)
            },
        )
        .await?;
        verify_artifact_hash(&download_path, &artifact)?;
        validate_archive_members(&download_path, &artifact)?;

        self.update_application_upgrade_progress(
            run,
            ApplicationUpgradeProgress {
                phase: phases::STAGING.to_string(),
                downloaded_bytes: artifact.size,
                total_bytes: artifact.size,
                ..ApplicationUpgradeProgress::checking(request)
            },
        )
        .await?;
        let extracted_dir = staging_dir.join("extracted");
        extract_archive(&download_path, &artifact, &extracted_dir)?;

        self.update_application_upgrade_progress(
            run,
            ApplicationUpgradeProgress {
                phase: phases::APPLYING.to_string(),
                downloaded_bytes: artifact.size,
                total_bytes: artifact.size,
                ..ApplicationUpgradeProgress::checking(request)
            },
        )
        .await?;
        let applied = apply_portable_upgrade(
            &extracted_dir,
            &artifact,
            request,
            SCRYER_VERSION,
            dependencies.ensure_available_space,
            dependencies.rename,
        )?;

        let journal = ApplicationUpgradeJournal {
            schema: JOURNAL_SCHEMA.to_string(),
            run_id: run.id.clone(),
            expected_version: request.expected_version.clone(),
            expected_tag: request.expected_tag.clone(),
            executable_path: applied.executable_path,
            backup_path: applied.backup_path,
            phase: phases::RESTARTING.to_string(),
            helper_error: None,
        };
        write_journal(&self.application_upgrade_journal_path(), &journal)?;
        self.update_application_upgrade_progress(
            run,
            ApplicationUpgradeProgress {
                phase: phases::RESTARTING.to_string(),
                downloaded_bytes: artifact.size,
                total_bytes: artifact.size,
                ..ApplicationUpgradeProgress::checking(request)
            },
        )
        .await?;
        let restart = self
            .runtime
            .jobs
            .application_upgrade_restart
            .read()
            .ok()
            .and_then(|handle| handle.clone())
            .ok_or_else(|| {
                AppError::Repository(
                    "application upgrade restart controller is not configured".to_string(),
                )
            })?;
        restart.schedule_restart();
        Ok(())
    }

    async fn update_application_upgrade_progress(
        &self,
        run: &mut JobRunRecord,
        progress: ApplicationUpgradeProgress,
    ) -> AppResult<()> {
        run.progress_json = serde_json::to_string(&progress).ok();
        run.updated_at = chrono::Utc::now();
        let updated = self.services.events.job_runs.update_job_run(run).await?;
        *run = updated.clone();
        self.runtime
            .jobs
            .job_run_tracker
            .upsert_active_run(JobRun::from_record(&updated, None))
            .await;
        Ok(())
    }

    async fn finish_application_upgrade_failure(
        &self,
        run: &mut JobRunRecord,
        actor: DomainEventActor,
        error_text: String,
    ) -> AppResult<()> {
        let mut progress = run
            .progress_json
            .as_deref()
            .and_then(|raw| serde_json::from_str::<ApplicationUpgradeProgress>(raw).ok())
            .unwrap_or(ApplicationUpgradeProgress {
                status: JobRunStatus::Running.as_str().to_string(),
                phase: phases::CHECKING.to_string(),
                downloaded_bytes: 0,
                total_bytes: 0,
                target_version: String::new(),
                target_tag: String::new(),
                error: None,
            });
        progress.status = JobRunStatus::Failed.as_str().to_string();
        progress.error = Some(error_text.clone());
        let now = chrono::Utc::now();
        run.status = JobRunStatus::Failed;
        run.progress_json = serde_json::to_string(&progress).ok();
        run.summary_text = Some("Application upgrade failed".to_string());
        run.error_text = Some(error_text.clone());
        run.completed_at = Some(now);
        run.updated_at = now;
        let updated = self.services.events.job_runs.update_job_run(run).await?;
        *run = updated.clone();
        self.runtime
            .jobs
            .job_run_tracker
            .upsert_active_run(JobRun::from_record(&updated, None))
            .await;
        let _ = self
            .append_domain_event(crate::domain_events::new_job_run_domain_event(
                actor,
                updated.id.clone(),
                DomainEventPayload::JobRunFailed(JobRunFailedEventData {
                    run_id: updated.id.clone(),
                    job_key: updated.job_key.as_str().to_string(),
                    error_text: Some(error_text),
                }),
            ))
            .await;
        Ok(())
    }

    /// Finalize the journal created before a restart and return runs that must
    /// remain running because an operating-system reboot is still required.
    pub async fn finalize_application_upgrade_journal(&self) -> AppResult<Vec<String>> {
        let journal_path = self.application_upgrade_journal_path();
        let Some(journal) = load_journal(&journal_path)? else {
            return Ok(Vec::new());
        };
        if journal.schema != JOURNAL_SCHEMA {
            return Err(AppError::Validation(format!(
                "unsupported application upgrade journal schema '{}'",
                journal.schema
            )));
        }
        if journal.phase == phases::REBOOT_REQUIRED {
            return Ok(vec![journal.run_id]);
        }
        if let Some(error) = journal.helper_error.clone() {
            self.finish_journal_application_upgrade(
                &journal,
                JobRunStatus::Failed,
                None,
                Some(error),
            )
            .await?;
            remove_file_if_exists(&journal_path)?;
            return Ok(Vec::new());
        }
        if journal.phase != phases::RESTARTING {
            return Err(AppError::Validation(format!(
                "unsupported application upgrade journal phase '{}'",
                journal.phase
            )));
        }

        let current_executable = std::env::current_exe().map_err(|error| {
            AppError::Repository(format!("failed to resolve running executable: {error}"))
        })?;
        if SCRYER_VERSION == journal.expected_version
            && current_executable == journal.executable_path
        {
            let old_version = self
                .services
                .events
                .job_runs
                .get_job_run(&journal.run_id)
                .await?
                .and_then(|run| {
                    run.operation_type
                        .split_once(':')
                        .and_then(|(_, versions)| versions.split_once("->"))
                        .map(|(old, _)| old.to_string())
                })
                .unwrap_or_else(|| "previous version".to_string());
            self.finish_journal_application_upgrade(
                &journal,
                JobRunStatus::Completed,
                Some(format!(
                    "Upgraded application from {old_version} to {}",
                    journal.expected_version
                )),
                None,
            )
            .await?;
            remove_file_if_exists(&journal.backup_path)?;
            remove_file_if_exists(&journal_path)?;
            remove_dir_if_exists(&self.application_upgrade_staging_dir())?;
            return Ok(Vec::new());
        }

        self.finish_journal_application_upgrade(
            &journal,
            JobRunStatus::Failed,
            None,
            Some("upgrade did not boot the expected version; backups preserved".to_string()),
        )
        .await?;
        Ok(Vec::new())
    }

    async fn finish_journal_application_upgrade(
        &self,
        journal: &ApplicationUpgradeJournal,
        status: JobRunStatus,
        summary_text: Option<String>,
        error_text: Option<String>,
    ) -> AppResult<()> {
        let mut run = self
            .services
            .events
            .job_runs
            .get_job_run(&journal.run_id)
            .await?
            .ok_or_else(|| {
                AppError::NotFound(format!("application upgrade run {}", journal.run_id))
            })?;
        let mut progress = run
            .progress_json
            .as_deref()
            .and_then(|raw| serde_json::from_str::<ApplicationUpgradeProgress>(raw).ok())
            .unwrap_or(ApplicationUpgradeProgress {
                status: JobRunStatus::Running.as_str().to_string(),
                phase: phases::RESTARTING.to_string(),
                downloaded_bytes: 0,
                total_bytes: 0,
                target_version: journal.expected_version.clone(),
                target_tag: journal.expected_tag.clone(),
                error: None,
            });
        progress.status = status.as_str().to_string();
        progress.error = error_text.clone();
        let now = chrono::Utc::now();
        run.status = status;
        run.progress_json = serde_json::to_string(&progress).ok();
        run.summary_text = summary_text.clone();
        run.error_text = error_text.clone();
        run.completed_at = Some(now);
        run.updated_at = now;
        let updated = self.services.events.job_runs.update_job_run(&run).await?;
        self.runtime
            .jobs
            .job_run_tracker
            .upsert_active_run(JobRun::from_record(&updated, None))
            .await;
        let payload = match status {
            JobRunStatus::Completed => {
                DomainEventPayload::JobRunCompleted(JobRunCompletedEventData {
                    run_id: updated.id.clone(),
                    job_key: updated.job_key.as_str().to_string(),
                    summary_text,
                })
            }
            JobRunStatus::Failed => DomainEventPayload::JobRunFailed(JobRunFailedEventData {
                run_id: updated.id.clone(),
                job_key: updated.job_key.as_str().to_string(),
                error_text,
            }),
            _ => unreachable!("journal finalization only writes terminal statuses"),
        };
        let _ = self
            .append_domain_event(crate::domain_events::new_job_run_domain_event(
                DomainEventActor::system(),
                updated.id.clone(),
                payload,
            ))
            .await;
        Ok(())
    }

    fn application_upgrade_root_dir(&self) -> PathBuf {
        self.runtime
            .environment
            .config_dir
            .as_ref()
            .join("application-upgrade")
    }

    fn application_upgrade_staging_dir(&self) -> PathBuf {
        self.application_upgrade_root_dir().join("staging")
    }

    fn application_upgrade_journal_path(&self) -> PathBuf {
        self.application_upgrade_root_dir().join("journal.json")
    }

    fn cleanup_application_upgrade_staging(&self) {
        if let Err(error) = remove_dir_if_exists(&self.application_upgrade_staging_dir()) {
            tracing::warn!(error = %error, "failed to clean application upgrade staging directory");
        }
    }
}

fn application_upgrade_http_client() -> AppResult<reqwest::Client> {
    reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::limited(5))
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|error| {
            AppError::Repository(format!("failed to build upgrade HTTP client: {error}"))
        })
}

async fn verify_upgrade_manifest_signature(
    manifest_raw: Vec<u8>,
    bundle_raw: Vec<u8>,
) -> AppResult<()> {
    verify_signed_blob(manifest_raw, bundle_raw, scryer_release_required_signer())
        .await
        .map_err(|error| {
            AppError::Validation(format!(
                "upgrade manifest signature verification failed: {error}"
            ))
        })
}

fn release_asset_url(tag: &str, filename: &str) -> AppResult<url::Url> {
    let mut url = url::Url::parse("https://github.com/scryer-media/scryer/releases/download/")
        .map_err(|error| AppError::Repository(format!("invalid release URL base: {error}")))?;
    url.path_segments_mut()
        .map_err(|_| {
            AppError::Repository("release URL base cannot accept path segments".to_string())
        })?
        .push(tag)
        .push(filename);
    Ok(url)
}

async fn fetch_capped_bytes(
    client: &reqwest::Client,
    url: &str,
    cap: u64,
    label: &str,
) -> AppResult<Vec<u8>> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| AppError::Repository(format!("failed to fetch {label}: {error}")))?
        .error_for_status()
        .map_err(|error| AppError::Repository(format!("failed to fetch {label}: {error}")))?;
    if response
        .content_length()
        .is_some_and(|content_length| content_length > cap)
    {
        return Err(AppError::Validation(format!(
            "{label} exceeds the maximum size of {cap} bytes"
        )));
    }

    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk
            .map_err(|error| AppError::Repository(format!("failed to read {label}: {error}")))?;
        let next_len = u64::try_from(bytes.len())
            .unwrap_or(u64::MAX)
            .saturating_add(u64::try_from(chunk.len()).unwrap_or(u64::MAX));
        if next_len > cap {
            return Err(AppError::Validation(format!(
                "{label} exceeds the maximum size of {cap} bytes"
            )));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn select_artifact(
    manifest: &UpgradeManifest,
    installation_kind: InstallationKind,
) -> AppResult<&UpgradeArtifact> {
    let platform = match std::env::consts::OS {
        "macos" => UpgradePlatform::Darwin,
        "linux" => UpgradePlatform::Linux,
        "windows" => UpgradePlatform::Windows,
        os => {
            return Err(AppError::Validation(format!(
                "no application upgrade artifact is available for operating system {os}"
            )));
        }
    };
    let arch = match std::env::consts::ARCH {
        "aarch64" => UpgradeArchitecture::Arm64,
        "x86_64" => UpgradeArchitecture::X86_64,
        arch => {
            return Err(AppError::Validation(format!(
                "no application upgrade artifact is available for architecture {arch}"
            )));
        }
    };
    let channel = match installation_kind {
        InstallationKind::Portable => UpgradeChannel::Portable,
        InstallationKind::DirectMsi => UpgradeChannel::Msi,
        _ => {
            return Err(AppError::Validation(
                "application upgrade installation is not eligible".to_string(),
            ));
        }
    };
    manifest
        .artifacts
        .iter()
        .find(|artifact| {
            artifact.platform == platform && artifact.arch == arch && artifact.channel == channel
        })
        .ok_or_else(|| {
            AppError::Validation("no upgrade artifact is available for this platform".to_string())
        })
}

async fn download_artifact(
    app: &AppUseCase,
    run: &mut JobRunRecord,
    request: &ApplicationUpgradeJobRequest,
    client: &reqwest::Client,
    artifact: &UpgradeArtifact,
    artifact_url_override: Option<&str>,
    destination: &Path,
) -> AppResult<()> {
    let response = client
        .get(artifact_url_override.unwrap_or(&artifact.url))
        .send()
        .await
        .map_err(|error| {
            AppError::Repository(format!("failed to download upgrade artifact: {error}"))
        })?
        .error_for_status()
        .map_err(|error| {
            AppError::Repository(format!("failed to download upgrade artifact: {error}"))
        })?;
    if response
        .content_length()
        .is_some_and(|content_length| content_length > artifact.size)
    {
        return Err(AppError::Validation(
            "upgrade artifact exceeds the manifest size".to_string(),
        ));
    }

    let mut file = tokio::fs::File::create(destination)
        .await
        .map_err(|error| {
            AppError::Repository(format!("failed to create upgrade staging file: {error}"))
        })?;
    let mut downloaded = 0_u64;
    let mut hasher = blake3::Hasher::new();
    let mut last_progress = Instant::now() - DOWNLOAD_PROGRESS_INTERVAL;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| {
            AppError::Repository(format!("failed to read upgrade artifact response: {error}"))
        })?;
        let next_downloaded =
            downloaded.saturating_add(u64::try_from(chunk.len()).unwrap_or(u64::MAX));
        if next_downloaded > artifact.size {
            return Err(AppError::Validation(
                "upgrade artifact exceeds the manifest size".to_string(),
            ));
        }
        file.write_all(&chunk).await.map_err(|error| {
            AppError::Repository(format!("failed to write upgrade staging file: {error}"))
        })?;
        hasher.update(&chunk);
        downloaded = next_downloaded;
        if last_progress.elapsed() >= DOWNLOAD_PROGRESS_INTERVAL {
            app.update_application_upgrade_progress(
                run,
                ApplicationUpgradeProgress {
                    phase: phases::DOWNLOADING.to_string(),
                    downloaded_bytes: downloaded,
                    total_bytes: artifact.size,
                    ..ApplicationUpgradeProgress::checking(request)
                },
            )
            .await?;
            last_progress = Instant::now();
        }
    }
    file.flush().await.map_err(|error| {
        AppError::Repository(format!("failed to flush upgrade staging file: {error}"))
    })?;
    if downloaded != artifact.size {
        return Err(AppError::Validation(format!(
            "upgrade artifact size mismatch: expected {} bytes, received {downloaded}",
            artifact.size
        )));
    }
    let expected_hash = blake3::Hash::from_hex(&artifact.blake3)
        .map_err(|error| AppError::Validation(format!("invalid manifest BLAKE3 hash: {error}")))?;
    if hasher.finalize() != expected_hash {
        return Err(AppError::Validation(
            "upgrade artifact BLAKE3 hash does not match the manifest".to_string(),
        ));
    }
    app.update_application_upgrade_progress(
        run,
        ApplicationUpgradeProgress {
            phase: phases::DOWNLOADING.to_string(),
            downloaded_bytes: downloaded,
            total_bytes: artifact.size,
            ..ApplicationUpgradeProgress::checking(request)
        },
    )
    .await
}

fn verify_artifact_hash(path: &Path, artifact: &UpgradeArtifact) -> AppResult<()> {
    let mut file = fs::File::open(path).map_err(|error| {
        AppError::Repository(format!("failed to open upgrade artifact: {error}"))
    })?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|error| {
            AppError::Repository(format!("failed to read upgrade artifact: {error}"))
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    if hasher.finalize().to_hex().as_str() != artifact.blake3 {
        return Err(AppError::Validation(
            "upgrade artifact BLAKE3 hash does not match the manifest".to_string(),
        ));
    }
    Ok(())
}

fn validate_archive_members(path: &Path, artifact: &UpgradeArtifact) -> AppResult<()> {
    match artifact.archive {
        UpgradeArchive::TarGz => validate_tar_members(path, artifact),
        UpgradeArchive::Zip => validate_zip_members(path, artifact),
        UpgradeArchive::Msi => Ok(()),
    }
}

fn validate_tar_members(path: &Path, artifact: &UpgradeArtifact) -> AppResult<()> {
    let file = fs::File::open(path).map_err(|error| {
        AppError::Repository(format!("failed to open upgrade archive: {error}"))
    })?;
    let mut archive = tar::Archive::new(GzDecoder::new(file));
    let mut actual = BTreeMap::new();
    for entry in archive.entries().map_err(archive_error)? {
        let entry = entry.map_err(archive_error)?;
        let member_path = archive_member_path(entry.path().map_err(archive_error)?.as_ref())?;
        if !entry.header().entry_type().is_file() {
            return Err(AppError::Validation(format!(
                "upgrade archive member '{member_path}' is not a regular file"
            )));
        }
        let size = entry.size();
        if actual.insert(member_path.clone(), size).is_some() {
            return Err(AppError::Validation(format!(
                "upgrade archive has duplicate member '{member_path}'"
            )));
        }
    }
    ensure_member_set_matches(&actual, artifact)
}

fn validate_zip_members(path: &Path, artifact: &UpgradeArtifact) -> AppResult<()> {
    let file = fs::File::open(path).map_err(|error| {
        AppError::Repository(format!("failed to open upgrade archive: {error}"))
    })?;
    let mut archive = zip::ZipArchive::new(file).map_err(archive_error)?;
    let mut actual = BTreeMap::new();
    for index in 0..archive.len() {
        let entry = archive.by_index(index).map_err(archive_error)?;
        let member_path = archive_member_path(Path::new(entry.name()))?;
        let mode_is_link = entry
            .unix_mode()
            .is_some_and(|mode| (mode & 0o170000) == 0o120000);
        if entry.is_dir() || mode_is_link {
            return Err(AppError::Validation(format!(
                "upgrade archive member '{member_path}' is not a regular file"
            )));
        }
        if actual.insert(member_path.clone(), entry.size()).is_some() {
            return Err(AppError::Validation(format!(
                "upgrade archive has duplicate member '{member_path}'"
            )));
        }
    }
    ensure_member_set_matches(&actual, artifact)
}

fn ensure_member_set_matches(
    actual: &BTreeMap<String, u64>,
    artifact: &UpgradeArtifact,
) -> AppResult<()> {
    let expected = artifact
        .members
        .iter()
        .map(|member| (member.path.clone(), member.size))
        .collect::<BTreeMap<_, _>>();
    if actual != &expected {
        return Err(AppError::Validation(
            "upgrade archive members do not exactly match the signed manifest".to_string(),
        ));
    }
    Ok(())
}

fn extract_archive(path: &Path, artifact: &UpgradeArtifact, destination: &Path) -> AppResult<()> {
    fs::create_dir_all(destination).map_err(|error| {
        AppError::Repository(format!(
            "failed to create extracted upgrade directory: {error}"
        ))
    })?;
    match artifact.archive {
        UpgradeArchive::TarGz => extract_tar(path, artifact, destination),
        UpgradeArchive::Zip => extract_zip(path, artifact, destination),
        UpgradeArchive::Msi => Ok(()),
    }
}

fn extract_tar(path: &Path, artifact: &UpgradeArtifact, destination: &Path) -> AppResult<()> {
    let file = fs::File::open(path).map_err(|error| {
        AppError::Repository(format!("failed to open upgrade archive: {error}"))
    })?;
    let mut archive = tar::Archive::new(GzDecoder::new(file));
    let expected = artifact_member_paths(artifact);
    for entry in archive.entries().map_err(archive_error)? {
        let mut entry = entry.map_err(archive_error)?;
        let member_path = archive_member_path(entry.path().map_err(archive_error)?.as_ref())?;
        let member = expected.get(&member_path).ok_or_else(|| {
            AppError::Validation(format!("unexpected upgrade archive member '{member_path}'"))
        })?;
        if !entry.header().entry_type().is_file() || entry.size() != member.size {
            return Err(AppError::Validation(format!(
                "invalid upgrade archive member '{member_path}'"
            )));
        }
        let output = destination.join(&member_path);
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent).map_err(archive_error)?;
        }
        let mut output_file = fs::File::create(&output).map_err(archive_error)?;
        std::io::copy(&mut entry, &mut output_file).map_err(archive_error)?;
        set_extracted_permissions(
            &output,
            entry.header().mode().unwrap_or(0o644),
            member.executable,
        )?;
    }
    Ok(())
}

fn extract_zip(path: &Path, artifact: &UpgradeArtifact, destination: &Path) -> AppResult<()> {
    let file = fs::File::open(path).map_err(|error| {
        AppError::Repository(format!("failed to open upgrade archive: {error}"))
    })?;
    let mut archive = zip::ZipArchive::new(file).map_err(archive_error)?;
    let expected = artifact_member_paths(artifact);
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(archive_error)?;
        let member_path = archive_member_path(Path::new(entry.name()))?;
        let member = expected.get(&member_path).ok_or_else(|| {
            AppError::Validation(format!("unexpected upgrade archive member '{member_path}'"))
        })?;
        let mode_is_link = entry
            .unix_mode()
            .is_some_and(|mode| (mode & 0o170000) == 0o120000);
        if entry.is_dir() || mode_is_link || entry.size() != member.size {
            return Err(AppError::Validation(format!(
                "invalid upgrade archive member '{member_path}'"
            )));
        }
        let output = destination.join(&member_path);
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent).map_err(archive_error)?;
        }
        let mut output_file = fs::File::create(&output).map_err(archive_error)?;
        std::io::copy(&mut entry, &mut output_file).map_err(archive_error)?;
        set_extracted_permissions(
            &output,
            entry.unix_mode().unwrap_or(0o644),
            member.executable,
        )?;
    }
    Ok(())
}

fn artifact_member_paths(
    artifact: &UpgradeArtifact,
) -> BTreeMap<String, crate::application_upgrade::manifest::UpgradeArtifactMember> {
    artifact
        .members
        .iter()
        .cloned()
        .map(|member| (member.path.clone(), member))
        .collect()
}

fn archive_member_path(path: &Path) -> AppResult<String> {
    let raw = path.to_string_lossy();
    let windows_drive_prefix = raw.as_bytes().get(1) == Some(&b':')
        && raw
            .as_bytes()
            .first()
            .is_some_and(|byte| byte.is_ascii_alphabetic());
    if path.is_absolute() || raw.starts_with('\\') || raw.contains('\\') || windows_drive_prefix {
        return Err(AppError::Validation(
            "upgrade archive contains an absolute member path".to_string(),
        ));
    }
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(component) => {
                components.push(component.to_string_lossy().to_string())
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(AppError::Validation(
                    "upgrade archive contains an unsafe member path".to_string(),
                ));
            }
        }
    }
    if components.is_empty() {
        return Err(AppError::Validation(
            "upgrade archive contains an empty member path".to_string(),
        ));
    }
    Ok(components.join("/"))
}

#[cfg(unix)]
fn set_extracted_permissions(path: &Path, mode: u32, executable: bool) -> AppResult<()> {
    use std::os::unix::fs::PermissionsExt;

    let mode = if executable {
        mode | 0o111
    } else {
        mode & !0o111
    };
    fs::set_permissions(path, fs::Permissions::from_mode(mode & 0o7777)).map_err(|error| {
        AppError::Repository(format!(
            "failed to set extracted upgrade permissions: {error}"
        ))
    })
}

#[cfg(not(unix))]
fn set_extracted_permissions(_path: &Path, _mode: u32, _executable: bool) -> AppResult<()> {
    Ok(())
}

fn apply_portable_upgrade(
    extracted_dir: &Path,
    artifact: &UpgradeArtifact,
    request: &ApplicationUpgradeJobRequest,
    current_version: &str,
    ensure_available_space: UpgradeSpaceCheck,
    rename: UpgradeRename,
) -> AppResult<AppliedPortableUpgrade> {
    #[cfg(unix)]
    {
        if request.installation_kind != InstallationKind::Portable {
            return Err(AppError::Validation(WINDOWS_APPLY_UNSUPPORTED.to_string()));
        }
        let executable_path = request
            .executable_path
            .clone()
            .or_else(|| std::env::current_exe().ok())
            .ok_or_else(|| {
                AppError::Repository("failed to resolve the running executable path".to_string())
            })?;
        let executable_dir = executable_path.parent().ok_or_else(|| {
            AppError::Validation("running executable has no parent directory".to_string())
        })?;
        let new_binary = find_upgraded_executable(extracted_dir, artifact, &executable_path)?;
        let new_binary_size = fs::metadata(&new_binary)
            .map_err(|error| {
                AppError::Repository(format!("failed to stat upgraded executable: {error}"))
            })?
            .len();
        ensure_available_space(
            executable_dir,
            new_binary_size.saturating_add(UPGRADE_STAGING_RESERVE_BYTES),
        )?;
        let new_path =
            executable_dir.join(format!(".scryer-upgrade-new-{}", request.expected_version));
        let backup_path = PathBuf::from(format!(
            "{}.pre-upgrade-{current_version}",
            executable_path.display()
        ));
        if backup_path.exists() {
            return Err(AppError::Validation(format!(
                "refusing to overwrite existing application backup '{}'",
                backup_path.display()
            )));
        }
        fs::copy(&new_binary, &new_path).map_err(|error| {
            AppError::Repository(format!("failed to stage replacement executable: {error}"))
        })?;
        if let Err(error) = rename(&executable_path, &backup_path) {
            let _ = fs::remove_file(&new_path);
            return Err(AppError::Repository(format!(
                "failed to retain current executable backup: {error}"
            )));
        }
        if let Err(error) = rename(&new_path, &executable_path) {
            let rollback_error = rename(&backup_path, &executable_path).err();
            let detail = rollback_error.map_or_else(String::new, |rollback| {
                format!("; rollback failed: {rollback}")
            });
            return Err(AppError::Repository(format!(
                "failed to replace application executable: {error}{detail}"
            )));
        }
        Ok(AppliedPortableUpgrade {
            executable_path,
            backup_path,
        })
    }
    #[cfg(not(unix))]
    {
        let _ = (
            extracted_dir,
            artifact,
            request,
            current_version,
            ensure_available_space,
            rename,
        );
        Err(AppError::Validation(WINDOWS_APPLY_UNSUPPORTED.to_string()))
    }
}

#[cfg(unix)]
fn find_upgraded_executable(
    extracted_dir: &Path,
    artifact: &UpgradeArtifact,
    executable_path: &Path,
) -> AppResult<PathBuf> {
    let current_name = executable_path.file_name();
    let exact = artifact
        .members
        .iter()
        .find(|member| member.executable && Path::new(&member.path).file_name() == current_name);
    let candidates = artifact
        .members
        .iter()
        .filter(|member| member.executable)
        .collect::<Vec<_>>();
    let selected = exact
        .or_else(|| (candidates.len() == 1).then_some(candidates[0]))
        .ok_or_else(|| {
            AppError::Validation(
                "upgrade archive does not identify a unique replacement executable".to_string(),
            )
        })?;
    Ok(extracted_dir.join(&selected.path))
}

fn recreate_staging_dir(path: &Path) -> AppResult<()> {
    remove_dir_if_exists(path)?;
    fs::create_dir_all(path).map_err(|error| {
        AppError::Repository(format!(
            "failed to create upgrade staging directory: {error}"
        ))
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|error| {
            AppError::Repository(format!(
                "failed to protect upgrade staging directory: {error}"
            ))
        })?;
    }
    Ok(())
}

fn ensure_available_space(path: &Path, required_bytes: u64) -> AppResult<()> {
    let space = filesystem_space_raw(path).map_err(|error| {
        AppError::Repository(format!(
            "failed to inspect upgrade filesystem space: {error}"
        ))
    })?;
    if space.available_bytes < required_bytes {
        return Err(AppError::Validation(format!(
            "insufficient free space for application upgrade: need {required_bytes} bytes, have {} bytes",
            space.available_bytes
        )));
    }
    Ok(())
}

fn write_journal(path: &Path, journal: &ApplicationUpgradeJournal) -> AppResult<()> {
    let parent = path.parent().ok_or_else(|| {
        AppError::Repository("application upgrade journal path has no parent directory".to_string())
    })?;
    fs::create_dir_all(parent).map_err(|error| {
        AppError::Repository(format!(
            "failed to create application upgrade journal directory: {error}"
        ))
    })?;
    let bytes = serde_json::to_vec(journal).map_err(|error| {
        AppError::Repository(format!(
            "failed to encode application upgrade journal: {error}"
        ))
    })?;
    let temporary = parent.join(format!(".journal-{}.tmp", journal.run_id));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary).map_err(|error| {
        AppError::Repository(format!(
            "failed to create application upgrade journal: {error}"
        ))
    })?;
    file.write_all(&bytes).map_err(|error| {
        AppError::Repository(format!(
            "failed to write application upgrade journal: {error}"
        ))
    })?;
    file.sync_all().map_err(|error| {
        AppError::Repository(format!(
            "failed to flush application upgrade journal: {error}"
        ))
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600)).map_err(|error| {
            AppError::Repository(format!(
                "failed to protect application upgrade journal: {error}"
            ))
        })?;
    }
    fs::rename(&temporary, path).map_err(|error| {
        AppError::Repository(format!(
            "failed to activate application upgrade journal: {error}"
        ))
    })
}

fn load_journal(path: &Path) -> AppResult<Option<ApplicationUpgradeJournal>> {
    let raw = match fs::read(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(AppError::Repository(format!(
                "failed to read application upgrade journal: {error}"
            )));
        }
    };
    serde_json::from_slice(&raw).map(Some).map_err(|error| {
        AppError::Validation(format!("invalid application upgrade journal: {error}"))
    })
}

fn remove_file_if_exists(path: &Path) -> AppResult<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(AppError::Repository(format!(
            "failed to remove application upgrade file '{}': {error}",
            path.display()
        ))),
    }
}

fn remove_dir_if_exists(path: &Path) -> AppResult<()> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(AppError::Repository(format!(
            "failed to remove application upgrade staging directory '{}': {error}",
            path.display()
        ))),
    }
}

fn archive_error(error: impl std::fmt::Display) -> AppError {
    AppError::Validation(format!("invalid upgrade archive: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::JobRunRepository;
    use crate::application_upgrade::manifest::{
        UPGRADE_MANIFEST_SCHEMA_VERSION, UpgradeArtifactMember,
    };
    use crate::application_upgrade::{ApplicationUpgradeRestartHandle, InstallationKind};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn archive_member_paths_reject_parent_components() {
        let error = archive_member_path(Path::new("bin/../scryer")).expect_err("unsafe path");
        assert!(error.to_string().contains("unsafe member path"));
    }

    #[test]
    fn archive_member_paths_reject_windows_paths_on_all_platforms() {
        for path in ["C:\\scryer", "bin\\scryer"] {
            let error = archive_member_path(Path::new(path)).expect_err("unsafe path");
            assert!(error.to_string().contains("absolute member path"));
        }
    }

    #[test]
    fn journal_round_trip_is_schema_stable() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("application-upgrade/journal.json");
        let journal = ApplicationUpgradeJournal {
            schema: JOURNAL_SCHEMA.to_string(),
            run_id: "run-1".to_string(),
            expected_version: "0.18.22".to_string(),
            expected_tag: "v0.18.22".to_string(),
            executable_path: PathBuf::from("/opt/scryer/scryer"),
            backup_path: PathBuf::from("/opt/scryer/scryer.pre-upgrade-0.18.21"),
            phase: phases::RESTARTING.to_string(),
            helper_error: None,
        };
        write_journal(&path, &journal).expect("write journal");
        assert_eq!(load_journal(&path).expect("load journal"), Some(journal));
    }

    fn portable_tar_artifact(size: u64) -> UpgradeArtifact {
        UpgradeArtifact {
            platform: UpgradePlatform::Linux,
            arch: UpgradeArchitecture::X86_64,
            channel: UpgradeChannel::Portable,
            asset_name: "scryer.tar.gz".to_string(),
            url: "https://github.com/scryer-media/scryer/releases/download/v0.18.22/scryer.tar.gz"
                .to_string(),
            size: 0,
            blake3: "0".repeat(64),
            archive: UpgradeArchive::TarGz,
            members: vec![
                crate::application_upgrade::manifest::UpgradeArtifactMember {
                    path: "scryer".to_string(),
                    size,
                    executable: true,
                },
            ],
        }
    }

    #[test]
    fn tar_archive_members_must_match_the_signed_manifest_exactly() {
        let temp = tempfile::tempdir().expect("tempdir");
        let archive_path = temp.path().join("upgrade.tar.gz");
        let output = fs::File::create(&archive_path).expect("create archive");
        let encoder = flate2::write::GzEncoder::new(output, flate2::Compression::default());
        let mut archive = tar::Builder::new(encoder);
        let bytes = b"new executable";
        let mut header = tar::Header::new_gnu();
        header.set_path("scryer").expect("set path");
        header.set_size(bytes.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        archive.append(&header, &bytes[..]).expect("append member");
        let encoder = archive.into_inner().expect("finish tar");
        encoder.finish().expect("finish gzip");

        let artifact = portable_tar_artifact(bytes.len() as u64);
        validate_archive_members(&archive_path, &artifact).expect("manifest member matches");

        let mismatch = portable_tar_artifact(bytes.len() as u64 + 1);
        let error = validate_archive_members(&archive_path, &mismatch)
            .expect_err("signed member size must match");
        assert!(error.to_string().contains("do not exactly match"));
    }

    #[cfg(unix)]
    fn test_request(executable_path: PathBuf) -> ApplicationUpgradeJobRequest {
        ApplicationUpgradeJobRequest {
            expected_tag: "v99.0.0".to_string(),
            expected_version: "99.0.0".to_string(),
            installation_kind: InstallationKind::Portable,
            executable_path: Some(executable_path),
        }
    }

    #[cfg(unix)]
    fn test_run(request: &ApplicationUpgradeJobRequest) -> JobRunRecord {
        let now = chrono::Utc::now();
        JobRunRecord {
            id: Id::new().0,
            job_key: JobKey::ApplicationUpgrade,
            operation_type: format!(
                "application_upgrade:{SCRYER_VERSION}->{}",
                request.expected_version
            ),
            status: JobRunStatus::Running,
            trigger_source: JobTriggerSource::Manual,
            actor_user_id: None,
            progress_json: serde_json::to_string(&ApplicationUpgradeProgress::checking(request))
                .ok(),
            summary_json: None,
            summary_text: None,
            error_text: None,
            started_at: now,
            completed_at: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[cfg(unix)]
    fn runtime_platform() -> UpgradePlatform {
        match std::env::consts::OS {
            "linux" => UpgradePlatform::Linux,
            "macos" => UpgradePlatform::Darwin,
            os => panic!("unsupported unix upgrade test platform {os}"),
        }
    }

    #[cfg(unix)]
    fn runtime_architecture() -> UpgradeArchitecture {
        match std::env::consts::ARCH {
            "x86_64" => UpgradeArchitecture::X86_64,
            "aarch64" => UpgradeArchitecture::Arm64,
            arch => panic!("unsupported upgrade test architecture {arch}"),
        }
    }

    #[cfg(unix)]
    fn tar_gz(members: &[(&str, &[u8], u32)]) -> Vec<u8> {
        let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        let mut archive = tar::Builder::new(encoder);
        for (path, bytes, mode) in members {
            let mut header = tar::Header::new_gnu();
            header.set_path(path).expect("set archive path");
            header.set_size(bytes.len() as u64);
            header.set_mode(*mode);
            header.set_cksum();
            archive
                .append(&header, *bytes)
                .expect("append archive member");
        }
        archive
            .into_inner()
            .expect("finish archive")
            .finish()
            .expect("finish gzip")
    }

    #[cfg(unix)]
    fn portable_manifest(bytes: &[u8], members: Vec<UpgradeArtifactMember>) -> UpgradeManifest {
        UpgradeManifest {
            schema: UPGRADE_MANIFEST_SCHEMA_VERSION.to_string(),
            tag: "v99.0.0".to_string(),
            version: "99.0.0".to_string(),
            artifacts: vec![UpgradeArtifact {
                platform: runtime_platform(),
                arch: runtime_architecture(),
                channel: UpgradeChannel::Portable,
                asset_name: "scryer.tar.gz".to_string(),
                url:
                    "https://github.com/scryer-media/scryer/releases/download/v99.0.0/scryer.tar.gz"
                        .to_string(),
                size: bytes.len() as u64,
                blake3: blake3::hash(bytes).to_hex().to_string(),
                archive: UpgradeArchive::TarGz,
                members,
            }],
        }
    }

    #[cfg(unix)]
    fn executable_member(bytes: &[u8]) -> UpgradeArtifactMember {
        UpgradeArtifactMember {
            path: "scryer".to_string(),
            size: bytes.len() as u64,
            executable: true,
        }
    }

    #[cfg(unix)]
    async fn artifact_server(body: Vec<u8>) -> (MockServer, String) {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/artifact"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(body))
            .mount(&server)
            .await;
        let url = format!("{}/artifact", server.uri());
        (server, url)
    }

    #[cfg(unix)]
    fn test_http_client() -> reqwest::Client {
        scryer_outbound_http::install_default_rustls_provider();
        reqwest::Client::new()
    }

    #[cfg(unix)]
    async fn run_pipeline_and_finish_failure(
        app: &AppUseCase,
        run: &mut JobRunRecord,
        request: &ApplicationUpgradeJobRequest,
        manifest: &UpgradeManifest,
        dependencies: UpgradePipelineDependencies<'_>,
    ) -> AppError {
        let error = app
            .run_upgrade_pipeline_with_dependencies(run, request, manifest, dependencies)
            .await
            .expect_err("pipeline should fail");
        app.cleanup_application_upgrade_staging();
        app.finish_application_upgrade_failure(run, DomainEventActor::system(), error.to_string())
            .await
            .expect("persist failed application upgrade run");
        error
    }

    #[cfg(unix)]
    fn injected_insufficient_space(_path: &Path, _required_bytes: u64) -> AppResult<()> {
        Err(AppError::Validation(
            "insufficient free space for application upgrade: injected test limit".to_string(),
        ))
    }

    #[cfg(unix)]
    fn fail_replacement_rename(from: &Path, to: &Path) -> std::io::Result<()> {
        if from
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with(".scryer-upgrade-new-"))
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "injected replacement rename failure",
            ));
        }
        fs::rename(from, to)
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn pipeline_happy_path_replaces_portable_executable_and_writes_restart_journal() {
        let temp = tempfile::tempdir().expect("tempdir");
        let executable_path = temp.path().join("bin/scryer");
        fs::create_dir_all(executable_path.parent().expect("executable parent"))
            .expect("create executable directory");
        fs::write(&executable_path, b"old executable").expect("write old executable");
        let new_binary = b"new executable";
        let archive = tar_gz(&[("scryer", new_binary, 0o755)]);
        let manifest = portable_manifest(&archive, vec![executable_member(new_binary)]);
        let (_server, artifact_url) = artifact_server(archive).await;
        let (app, _actor, job_runs) =
            crate::lib_tests::bootstrap_application_upgrade(temp.path().join("data"));
        let restarted = Arc::new(AtomicBool::new(false));
        let restart_observed = Arc::clone(&restarted);
        app.set_application_upgrade_restart_handle(ApplicationUpgradeRestartHandle::new(
            move || {
                restart_observed.store(true, Ordering::SeqCst);
            },
        ));
        let request = test_request(executable_path.clone());
        let mut run = test_run(&request);
        job_runs.seed(run.clone()).await;
        let client = test_http_client();

        app.run_upgrade_pipeline_with_dependencies(
            &mut run,
            &request,
            &manifest,
            UpgradePipelineDependencies {
                client: &client,
                artifact_url_override: Some(&artifact_url),
                ensure_available_space,
                rename: rename_path,
            },
        )
        .await
        .expect("portable upgrade pipeline succeeds");

        assert_eq!(
            fs::read(&executable_path).expect("replacement executable"),
            new_binary
        );
        let backup_path = PathBuf::from(format!(
            "{}.pre-upgrade-{SCRYER_VERSION}",
            executable_path.display()
        ));
        assert_eq!(
            fs::read(&backup_path).expect("backup executable"),
            b"old executable"
        );
        let journal = load_journal(&app.application_upgrade_journal_path())
            .expect("load journal")
            .expect("journal exists");
        assert_eq!(journal.phase, phases::RESTARTING);
        assert_eq!(journal.executable_path, executable_path);
        assert_eq!(journal.backup_path, backup_path);
        let progress: ApplicationUpgradeProgress =
            serde_json::from_str(run.progress_json.as_deref().expect("running progress"))
                .expect("decode progress");
        assert_eq!(run.status, JobRunStatus::Running);
        assert_eq!(progress.status, JobRunStatus::Running.as_str());
        assert_eq!(progress.phase, phases::RESTARTING);
        assert!(restarted.load(Ordering::SeqCst));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn pipeline_blake3_mismatch_fails_and_cleans_staging_without_touching_executable() {
        let temp = tempfile::tempdir().expect("tempdir");
        let executable_path = temp.path().join("bin/scryer");
        fs::create_dir_all(executable_path.parent().expect("executable parent"))
            .expect("create executable directory");
        fs::write(&executable_path, b"old executable").expect("write old executable");
        let new_binary = b"new executable";
        let archive = tar_gz(&[("scryer", new_binary, 0o755)]);
        let mut manifest = portable_manifest(&archive, vec![executable_member(new_binary)]);
        manifest.artifacts[0].blake3 = "0".repeat(64);
        let (_server, artifact_url) = artifact_server(archive).await;
        let (app, _actor, job_runs) =
            crate::lib_tests::bootstrap_application_upgrade(temp.path().join("data"));
        let request = test_request(executable_path.clone());
        let mut run = test_run(&request);
        job_runs.seed(run.clone()).await;
        let client = test_http_client();

        let error = run_pipeline_and_finish_failure(
            &app,
            &mut run,
            &request,
            &manifest,
            UpgradePipelineDependencies {
                client: &client,
                artifact_url_override: Some(&artifact_url),
                ensure_available_space,
                rename: rename_path,
            },
        )
        .await;

        assert!(error.to_string().contains("BLAKE3 hash does not match"));
        assert_eq!(run.status, JobRunStatus::Failed);
        assert!(!app.application_upgrade_staging_dir().exists());
        assert_eq!(
            fs::read(&executable_path).expect("original executable"),
            b"old executable"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn pipeline_oversize_response_fails_with_manifest_size_error() {
        let temp = tempfile::tempdir().expect("tempdir");
        let executable_path = temp.path().join("bin/scryer");
        fs::create_dir_all(executable_path.parent().expect("executable parent"))
            .expect("create executable directory");
        fs::write(&executable_path, b"old executable").expect("write old executable");
        let new_binary = b"new executable";
        let archive = tar_gz(&[("scryer", new_binary, 0o755)]);
        let mut manifest = portable_manifest(&archive, vec![executable_member(new_binary)]);
        manifest.artifacts[0].size = manifest.artifacts[0].size.saturating_sub(1);
        let (_server, artifact_url) = artifact_server(archive).await;
        let (app, _actor, job_runs) =
            crate::lib_tests::bootstrap_application_upgrade(temp.path().join("data"));
        let request = test_request(executable_path.clone());
        let mut run = test_run(&request);
        job_runs.seed(run.clone()).await;
        let client = test_http_client();

        let error = run_pipeline_and_finish_failure(
            &app,
            &mut run,
            &request,
            &manifest,
            UpgradePipelineDependencies {
                client: &client,
                artifact_url_override: Some(&artifact_url),
                ensure_available_space,
                rename: rename_path,
            },
        )
        .await;

        assert!(error.to_string().contains("exceeds the manifest size"));
        assert_eq!(run.status, JobRunStatus::Failed);
        assert!(!app.application_upgrade_staging_dir().exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn pipeline_archive_member_mismatch_fails_before_apply() {
        let temp = tempfile::tempdir().expect("tempdir");
        let executable_path = temp.path().join("bin/scryer");
        fs::create_dir_all(executable_path.parent().expect("executable parent"))
            .expect("create executable directory");
        fs::write(&executable_path, b"old executable").expect("write old executable");
        let new_binary = b"new executable";
        let archive = tar_gz(&[
            ("scryer", new_binary, 0o755),
            ("unexpected.txt", b"extra member", 0o644),
        ]);
        let manifest = portable_manifest(&archive, vec![executable_member(new_binary)]);
        let (_server, artifact_url) = artifact_server(archive).await;
        let (app, _actor, job_runs) =
            crate::lib_tests::bootstrap_application_upgrade(temp.path().join("data"));
        let request = test_request(executable_path.clone());
        let mut run = test_run(&request);
        job_runs.seed(run.clone()).await;
        let client = test_http_client();

        let error = run_pipeline_and_finish_failure(
            &app,
            &mut run,
            &request,
            &manifest,
            UpgradePipelineDependencies {
                client: &client,
                artifact_url_override: Some(&artifact_url),
                ensure_available_space,
                rename: rename_path,
            },
        )
        .await;

        assert!(error.to_string().contains("members do not exactly match"));
        assert_eq!(run.status, JobRunStatus::Failed);
        assert_eq!(
            fs::read(&executable_path).expect("original executable"),
            b"old executable"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn pipeline_insufficient_space_uses_injected_space_check() {
        let temp = tempfile::tempdir().expect("tempdir");
        let executable_path = temp.path().join("bin/scryer");
        fs::create_dir_all(executable_path.parent().expect("executable parent"))
            .expect("create executable directory");
        fs::write(&executable_path, b"old executable").expect("write old executable");
        let new_binary = b"new executable";
        let archive = tar_gz(&[("scryer", new_binary, 0o755)]);
        let manifest = portable_manifest(&archive, vec![executable_member(new_binary)]);
        let (app, _actor, job_runs) =
            crate::lib_tests::bootstrap_application_upgrade(temp.path().join("data"));
        let request = test_request(executable_path);
        let mut run = test_run(&request);
        job_runs.seed(run.clone()).await;
        let client = test_http_client();

        let error = run_pipeline_and_finish_failure(
            &app,
            &mut run,
            &request,
            &manifest,
            UpgradePipelineDependencies {
                client: &client,
                artifact_url_override: None,
                ensure_available_space: injected_insufficient_space,
                rename: rename_path,
            },
        )
        .await;

        assert!(
            error
                .to_string()
                .contains("insufficient free space for application upgrade")
        );
        assert_eq!(run.status, JobRunStatus::Failed);
        assert!(!app.application_upgrade_staging_dir().exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn pipeline_apply_rename_failure_restores_original_executable() {
        let temp = tempfile::tempdir().expect("tempdir");
        let executable_path = temp.path().join("bin/scryer");
        fs::create_dir_all(executable_path.parent().expect("executable parent"))
            .expect("create executable directory");
        fs::write(&executable_path, b"old executable").expect("write old executable");
        let new_binary = b"new executable";
        let archive = tar_gz(&[("scryer", new_binary, 0o755)]);
        let manifest = portable_manifest(&archive, vec![executable_member(new_binary)]);
        let (_server, artifact_url) = artifact_server(archive).await;
        let (app, _actor, job_runs) =
            crate::lib_tests::bootstrap_application_upgrade(temp.path().join("data"));
        let request = test_request(executable_path.clone());
        let mut run = test_run(&request);
        job_runs.seed(run.clone()).await;
        let client = test_http_client();

        let error = run_pipeline_and_finish_failure(
            &app,
            &mut run,
            &request,
            &manifest,
            UpgradePipelineDependencies {
                client: &client,
                artifact_url_override: Some(&artifact_url),
                ensure_available_space,
                rename: fail_replacement_rename,
            },
        )
        .await;

        let backup_path = PathBuf::from(format!(
            "{}.pre-upgrade-{SCRYER_VERSION}",
            executable_path.display()
        ));
        assert!(
            error
                .to_string()
                .contains("failed to replace application executable")
        );
        assert_eq!(run.status, JobRunStatus::Failed);
        assert_eq!(
            fs::read(&executable_path).expect("rolled-back executable"),
            b"old executable"
        );
        assert!(!backup_path.exists(), "backup should have been rolled back");
    }

    #[tokio::test]
    async fn tampered_signature_is_rejected_by_the_real_sigstore_verifier() {
        let error = verify_upgrade_manifest_signature(
            b"{\"schema\":\"scryer.upgrade.manifest.v1\"}".to_vec(),
            b"not a sigstore bundle".to_vec(),
        )
        .await
        .expect_err("garbage signature bundle must be rejected");
        assert!(
            error
                .to_string()
                .contains("upgrade manifest signature verification failed")
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn journal_finalization_completes_matching_boot_and_cleans_recovery_files() {
        let temp = tempfile::tempdir().expect("tempdir");
        let (app, _actor, job_runs) =
            crate::lib_tests::bootstrap_application_upgrade(temp.path().join("data"));
        let executable_path = std::env::current_exe().expect("current executable");
        let request = test_request(executable_path.clone());
        let run = test_run(&request);
        job_runs.seed(run.clone()).await;
        let backup_path = temp.path().join("scryer.pre-upgrade");
        fs::write(&backup_path, b"backup").expect("write backup");
        let staging_file = app.application_upgrade_staging_dir().join("artifact");
        fs::create_dir_all(staging_file.parent().expect("staging parent")).expect("create staging");
        fs::write(&staging_file, b"staged artifact").expect("write staging");
        write_journal(
            &app.application_upgrade_journal_path(),
            &ApplicationUpgradeJournal {
                schema: JOURNAL_SCHEMA.to_string(),
                run_id: run.id.clone(),
                expected_version: SCRYER_VERSION.to_string(),
                expected_tag: request.expected_tag.clone(),
                executable_path,
                backup_path: backup_path.clone(),
                phase: phases::RESTARTING.to_string(),
                helper_error: None,
            },
        )
        .expect("write journal");

        assert!(
            app.finalize_application_upgrade_journal()
                .await
                .expect("finalize journal")
                .is_empty()
        );

        let finalized = job_runs
            .get_job_run(&run.id)
            .await
            .expect("load finalized run")
            .expect("run exists");
        assert_eq!(finalized.status, JobRunStatus::Completed);
        assert!(!backup_path.exists());
        assert!(!app.application_upgrade_journal_path().exists());
        assert!(!app.application_upgrade_staging_dir().exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn journal_finalization_records_helper_error_and_preserves_backup() {
        let temp = tempfile::tempdir().expect("tempdir");
        let (app, _actor, job_runs) =
            crate::lib_tests::bootstrap_application_upgrade(temp.path().join("data"));
        let request = test_request(temp.path().join("bin/scryer"));
        let run = test_run(&request);
        job_runs.seed(run.clone()).await;
        let backup_path = temp.path().join("scryer.pre-upgrade");
        fs::write(&backup_path, b"backup").expect("write backup");
        write_journal(
            &app.application_upgrade_journal_path(),
            &ApplicationUpgradeJournal {
                schema: JOURNAL_SCHEMA.to_string(),
                run_id: run.id.clone(),
                expected_version: request.expected_version.clone(),
                expected_tag: request.expected_tag.clone(),
                executable_path: request.executable_path.clone().expect("executable path"),
                backup_path: backup_path.clone(),
                phase: phases::RESTARTING.to_string(),
                helper_error: Some("elevation helper failed".to_string()),
            },
        )
        .expect("write journal");

        app.finalize_application_upgrade_journal()
            .await
            .expect("finalize helper failure");

        let finalized = job_runs
            .get_job_run(&run.id)
            .await
            .expect("load finalized run")
            .expect("run exists");
        assert_eq!(finalized.status, JobRunStatus::Failed);
        assert_eq!(
            finalized.error_text.as_deref(),
            Some("elevation helper failed")
        );
        assert!(backup_path.exists());
        assert!(!app.application_upgrade_journal_path().exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn journal_finalization_preserves_files_when_boot_version_mismatches() {
        let temp = tempfile::tempdir().expect("tempdir");
        let (app, _actor, job_runs) =
            crate::lib_tests::bootstrap_application_upgrade(temp.path().join("data"));
        let request = test_request(temp.path().join("bin/scryer"));
        let run = test_run(&request);
        job_runs.seed(run.clone()).await;
        let backup_path = temp.path().join("scryer.pre-upgrade");
        fs::write(&backup_path, b"backup").expect("write backup");
        write_journal(
            &app.application_upgrade_journal_path(),
            &ApplicationUpgradeJournal {
                schema: JOURNAL_SCHEMA.to_string(),
                run_id: run.id.clone(),
                expected_version: "0.0.0".to_string(),
                expected_tag: request.expected_tag.clone(),
                executable_path: request.executable_path.clone().expect("executable path"),
                backup_path: backup_path.clone(),
                phase: phases::RESTARTING.to_string(),
                helper_error: None,
            },
        )
        .expect("write journal");

        app.finalize_application_upgrade_journal()
            .await
            .expect("finalize mismatch");

        let finalized = job_runs
            .get_job_run(&run.id)
            .await
            .expect("load finalized run")
            .expect("run exists");
        assert_eq!(finalized.status, JobRunStatus::Failed);
        assert!(
            finalized
                .error_text
                .as_deref()
                .is_some_and(|error| error.contains("backups preserved"))
        );
        assert!(backup_path.exists());
        assert!(app.application_upgrade_journal_path().exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn journal_finalization_leaves_reboot_required_run_untouched() {
        let temp = tempfile::tempdir().expect("tempdir");
        let (app, _actor, job_runs) =
            crate::lib_tests::bootstrap_application_upgrade(temp.path().join("data"));
        let request = test_request(temp.path().join("bin/scryer"));
        let run = test_run(&request);
        job_runs.seed(run.clone()).await;
        let backup_path = temp.path().join("scryer.pre-upgrade");
        fs::write(&backup_path, b"backup").expect("write backup");
        write_journal(
            &app.application_upgrade_journal_path(),
            &ApplicationUpgradeJournal {
                schema: JOURNAL_SCHEMA.to_string(),
                run_id: run.id.clone(),
                expected_version: request.expected_version.clone(),
                expected_tag: request.expected_tag.clone(),
                executable_path: request.executable_path.clone().expect("executable path"),
                backup_path: backup_path.clone(),
                phase: phases::REBOOT_REQUIRED.to_string(),
                helper_error: None,
            },
        )
        .expect("write journal");

        assert_eq!(
            app.finalize_application_upgrade_journal()
                .await
                .expect("finalize reboot journal"),
            vec![run.id.clone()]
        );

        let unchanged = job_runs
            .get_job_run(&run.id)
            .await
            .expect("load run")
            .expect("run exists");
        assert_eq!(unchanged.status, JobRunStatus::Running);
        assert!(backup_path.exists());
        assert!(app.application_upgrade_journal_path().exists());
    }
}
