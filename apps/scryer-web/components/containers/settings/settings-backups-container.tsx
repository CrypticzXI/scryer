import * as React from "react";
import { Download, Eye, EyeOff, Loader2, LockKeyhole, Plus, Trash2 } from "lucide-react";
import { useBeforeUnload, useBlocker } from "react-router-dom";
import { useClient } from "urql";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { InfoHelp } from "@/components/common/info-help";
import { SettingsToggleSwitch } from "@/components/common/settings-toggle-switch";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { TimePicker } from "@/components/ui/time-picker";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import {
  createBackupMutation,
  deleteBackupMutation,
  prepareBackupDownloadMutation,
  updateAutoBackupSettingsMutation,
} from "@/lib/graphql/mutations";
import { autoBackupSettingsQuery, backupsQuery } from "@/lib/graphql/queries";
import { scryerFetch } from "@/lib/graphql/urql-client";
import { useSettingsSubscription } from "@/lib/hooks/use-settings-subscription";
import { getRuntimeBasePath } from "@/lib/runtime-config";
import { cn } from "@/lib/utils";
import {
  boxedActionButtonBaseClass,
  boxedActionButtonToneClass,
} from "@/lib/utils/action-button-styles";
import type { AutoBackupSettings } from "@/lib/types/settings";

type BackupRowCount = {
  table: string;
  rowCount: string;
};

type BackupTrigger = "manual" | "auto";

type BackupInfoRecord = {
  filename: string;
  sizeBytes: string;
  createdAt: string;
  formatVersion: string;
  sourceEngine: string;
  sourceMigrationKey: string | null;
  trigger: BackupTrigger;
  encrypted: boolean;
  rowCounts: BackupRowCount[];
  status: "creating" | "ready" | "invalid" | "failed";
  errorMessage: string | null;
};

type BackupsQueryResult = {
  backups?: BackupInfoRecord[];
};

type CreateBackupMutationResult = {
  createBackup?: BackupInfoRecord;
};

type DeleteBackupMutationResult = {
  deleteBackup?: boolean;
};

type PrepareBackupDownloadMutationResult = {
  prepareBackupDownload?: {
    downloadUrl: string;
    downloadAuthorizationToken: string;
    expiresAt: string;
  };
};

type AutoBackupSettingsQueryResult = {
  autoBackupSettings?: AutoBackupSettings;
};

type UpdateAutoBackupSettingsMutationResult = {
  updateAutoBackupSettings?: AutoBackupSettings;
};

const DEFAULT_AUTO_BACKUP_SETTINGS: AutoBackupSettings = {
  enabled: false,
  dailyTimeLocal: "03:00",
  autoBackupKeyPresent: false,
  autoBackupDisabledMissingKeyNotice: false,
  nextRunAt: null,
};
const UNSAVED_AUTO_BACKUP_CHANGES_MESSAGE =
  "You have unsaved automatic backup changes. Leave without saving?";

type SaveFilePickerWindow = Window & {
  showSaveFilePicker?: (options: {
    suggestedName: string;
  }) => Promise<{
    createWritable: () => Promise<WritableStream<Uint8Array>>;
  }>;
};

function mutationErrorMessage(error: unknown, fallback: string): string {
  if (error instanceof Error && error.message.trim()) {
    return error.message.trim();
  }
  return fallback;
}

function buildAppUrl(path: string): string {
  const basePath = getRuntimeBasePath();
  return basePath === "/" ? path : `${basePath}${path}`;
}

async function readResponseErrorMessage(response: Response, fallback: string): Promise<string> {
  const contentType = response.headers.get("content-type") ?? "";
  if (contentType.includes("application/json")) {
    const payload = await response.json().catch(() => null) as {
      error?: string;
      error_id?: string;
    } | null;
    const message = payload?.error?.trim();
    if (message) {
      const errorId = payload?.error_id?.trim();
      return errorId ? `${message}. Reference ID: ${errorId}` : message;
    }
  }

  const text = await response.text().catch(() => "");
  return text.trim() || fallback;
}

async function saveDownloadResponse(response: Response, filename: string): Promise<void> {
  const windowWithPicker = window as SaveFilePickerWindow;
  if (response.body && typeof windowWithPicker.showSaveFilePicker === "function") {
    try {
      const handle = await windowWithPicker.showSaveFilePicker({
        suggestedName: filename,
      });
      const writable = await handle.createWritable();
      await response.body.pipeTo(writable);
      return;
    } catch (error) {
      if (error instanceof DOMException && error.name === "AbortError") {
        return;
      }
      throw error;
    }
  }

  const blob = await response.blob();
  const downloadUrl = window.URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = downloadUrl;
  link.download = filename;
  document.body.append(link);
  link.click();
  link.remove();
  window.setTimeout(() => {
    window.URL.revokeObjectURL(downloadUrl);
  }, 0);
}

function sortBackups(backups: BackupInfoRecord[]): BackupInfoRecord[] {
  return [...backups].sort((left, right) => {
    const leftTime = Date.parse(left.createdAt);
    const rightTime = Date.parse(right.createdAt);
    if (Number.isFinite(leftTime) && Number.isFinite(rightTime) && leftTime !== rightTime) {
      return rightTime - leftTime;
    }
    return right.filename.localeCompare(left.filename);
  });
}

function upsertBackup(backups: BackupInfoRecord[], nextBackup: BackupInfoRecord): BackupInfoRecord[] {
  return sortBackups([
    nextBackup,
    ...backups.filter((backup) => backup.filename !== nextBackup.filename),
  ]);
}

function autoBackupSettingsEqual(
  left: AutoBackupSettings,
  right: AutoBackupSettings,
): boolean {
  return (
    left.enabled === right.enabled &&
    left.dailyTimeLocal === right.dailyTimeLocal &&
    left.autoBackupKeyPresent === right.autoBackupKeyPresent &&
    left.autoBackupDisabledMissingKeyNotice ===
      right.autoBackupDisabledMissingKeyNotice &&
    (left.nextRunAt ?? null) === (right.nextRunAt ?? null)
  );
}

function formatBytes(sizeBytes: string): string {
  const value = Number(sizeBytes);
  if (!Number.isFinite(value) || value < 0) {
    return sizeBytes;
  }

  if (value < 1024) {
    return `${value} B`;
  }

  const units = ["KB", "MB", "GB", "TB"];
  let current = value / 1024;
  let unitIndex = 0;
  while (current >= 1024 && unitIndex < units.length - 1) {
    current /= 1024;
    unitIndex += 1;
  }
  return `${current.toFixed(current >= 100 ? 0 : current >= 10 ? 1 : 2)} ${units[unitIndex]}`;
}

function formatDateTime(value: string): string {
  const timestamp = Date.parse(value);
  if (!Number.isFinite(timestamp)) {
    return value;
  }
  return new Date(timestamp).toLocaleString();
}

function statusTone(status: BackupInfoRecord["status"]): string {
  switch (status) {
    case "creating":
      return "border-amber-500/30 bg-amber-500/10 text-amber-300";
    case "invalid":
    case "failed":
      return "border-destructive/40 bg-destructive/10 text-destructive";
    case "ready":
    default:
      return "border-emerald-500/30 bg-emerald-500/10 text-emerald-300";
  }
}

function BackupStatusBadge({
  status,
  label,
}: {
  status: BackupInfoRecord["status"];
  label: string;
}) {
  return (
    <span
      className={`inline-flex items-center gap-2 rounded-full border px-2.5 py-1 text-xs font-medium ${statusTone(status)}`}
    >
      {status === "creating" ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : null}
      {label}
    </span>
  );
}

export function SettingsBackupsContainer() {
  const client = useClient();
  const t = useTranslate();
  const setGlobalStatus = useGlobalStatus();

  const [backups, setBackups] = React.useState<BackupInfoRecord[]>([]);
  const [loading, setLoading] = React.useState(true);
  const [savedAutoBackupSettings, setSavedAutoBackupSettings] =
    React.useState<AutoBackupSettings>(DEFAULT_AUTO_BACKUP_SETTINGS);
  const [autoBackupSettings, setAutoBackupSettings] =
    React.useState<AutoBackupSettings>(DEFAULT_AUTO_BACKUP_SETTINGS);
  const [autoBackupLoading, setAutoBackupLoading] = React.useState(true);
  const [autoBackupSaving, setAutoBackupSaving] = React.useState(false);
  const [autoBackupExpanded, setAutoBackupExpanded] = React.useState(false);
  const [autoBackupKey, setAutoBackupKey] = React.useState("");
  const [clearAutoBackupKey, setClearAutoBackupKey] = React.useState(false);
  const [showAutoBackupKey, setShowAutoBackupKey] = React.useState(false);
  const [createDialogOpen, setCreateDialogOpen] = React.useState(false);
  const [password, setPassword] = React.useState("");
  const [confirmPassword, setConfirmPassword] = React.useState("");
  const [creatingRequest, setCreatingRequest] = React.useState(false);
  const [pendingDelete, setPendingDelete] = React.useState<BackupInfoRecord | null>(null);
  const [deletingFilename, setDeletingFilename] = React.useState<string | null>(null);
  const [downloadingFilename, setDownloadingFilename] = React.useState<string | null>(null);
  const hasCreatingManualBackup = backups.some(
    (backup) => backup.status === "creating" && backup.trigger === "manual",
  );
  const passwordRequired = password.trim().length === 0;
  const confirmPasswordRequired = confirmPassword.length === 0;
  const passwordMismatch = confirmPassword.length > 0 && password !== confirmPassword;
  const autoBackupWillHaveKey =
    !clearAutoBackupKey &&
    (autoBackupKey.trim().length > 0 || autoBackupSettings.autoBackupKeyPresent);
  const autoBackupKeyRequired = autoBackupSettings.enabled && !autoBackupWillHaveKey;
  const canSaveAutoBackupSettings = !autoBackupSaving && !autoBackupKeyRequired;
  const canCreateBackup =
    !creatingRequest &&
    !hasCreatingManualBackup &&
    !passwordRequired &&
    !confirmPasswordRequired &&
    !passwordMismatch;
  const pageLoading = loading || autoBackupLoading;
  const autoBackupNextRunLabel =
    autoBackupSettings.enabled && autoBackupSettings.nextRunAt
      ? formatDateTime(autoBackupSettings.nextRunAt)
      : t("label.disabled");
  const autoBackupKeyPlaceholder = clearAutoBackupKey
    ? ""
    : autoBackupSettings.autoBackupKeyPresent
      ? t("settings.autoBackupsKeyAlreadySetHint")
      : t("settings.autoBackupsSetKey");
  const autoBackupTriggerLabel = (trigger: BackupTrigger) =>
    trigger === "auto" ? t("settings.backupsAutomatic") : t("settings.backupsManual");
  const autoBackupDirty =
    !autoBackupSettingsEqual(autoBackupSettings, savedAutoBackupSettings) ||
    autoBackupKey.length > 0 ||
    clearAutoBackupKey;
  const shouldBlockNavigation = autoBackupDirty && !autoBackupSaving;
  const autoBackupNavigationBlocker = useBlocker(shouldBlockNavigation);

  useBeforeUnload(
    React.useCallback((event: BeforeUnloadEvent) => {
      if (!shouldBlockNavigation) {
        return;
      }
      event.preventDefault();
      event.returnValue = "";
    }, [shouldBlockNavigation]),
  );

  const fetchBackups = React.useCallback(async () => {
    try {
      const { data, error } = await client
        .query<BackupsQueryResult>(backupsQuery, {}, { requestPolicy: "network-only" })
        .toPromise();
      if (error) {
        throw error;
      }
      setBackups(sortBackups(data?.backups ?? []));
    } catch (error) {
      setGlobalStatus(mutationErrorMessage(error, t("status.failedToLoad")));
    } finally {
      setLoading(false);
    }
  }, [client, setGlobalStatus, t]);

  const fetchAutoBackupSettings = React.useCallback(async () => {
    try {
      const { data, error } = await client
        .query<AutoBackupSettingsQueryResult>(
          autoBackupSettingsQuery,
          {},
          { requestPolicy: "network-only" },
        )
        .toPromise();
      if (error) {
        throw error;
      }
      const nextSettings = data?.autoBackupSettings ?? DEFAULT_AUTO_BACKUP_SETTINGS;
      setSavedAutoBackupSettings(nextSettings);
      setAutoBackupSettings(nextSettings);
    } catch (error) {
      setGlobalStatus(mutationErrorMessage(error, t("status.failedToLoad")));
    } finally {
      setAutoBackupLoading(false);
    }
  }, [client, setGlobalStatus, t]);

  React.useEffect(() => {
    void fetchBackups();
  }, [fetchBackups]);

  React.useEffect(() => {
    void fetchAutoBackupSettings();
  }, [fetchAutoBackupSettings]);

  React.useEffect(() => {
    if (autoBackupSettings.enabled) {
      setAutoBackupExpanded(true);
      return;
    }

    setAutoBackupExpanded(false);
  }, [autoBackupSettings.enabled]);

  useSettingsSubscription(
    React.useCallback(
      (keys: string[]) => {
        if (keys.includes("backup")) {
          void fetchBackups();
        }
      },
      [fetchBackups],
    ),
  );

  React.useEffect(() => {
    if (!backups.some((backup) => backup.status === "creating")) {
      return;
    }

    const intervalId = window.setInterval(() => {
      void fetchBackups();
    }, 2000);

    return () => window.clearInterval(intervalId);
  }, [backups, fetchBackups]);

  React.useEffect(() => {
    if (autoBackupNavigationBlocker.state !== "blocked") {
      return;
    }

    if (window.confirm(UNSAVED_AUTO_BACKUP_CHANGES_MESSAGE)) {
      autoBackupNavigationBlocker.proceed();
      return;
    }

    autoBackupNavigationBlocker.reset();
  }, [autoBackupNavigationBlocker]);

  const handleCreateBackup = React.useCallback(async () => {
    if (!canCreateBackup) {
      return;
    }

    setCreatingRequest(true);
    try {
      const nextPassword = password;
      const { data, error } = await client
        .mutation<CreateBackupMutationResult>(createBackupMutation, {
          password: nextPassword,
        })
        .toPromise();
      if (error || !data?.createBackup) {
        throw error ?? new Error(t("status.failedToUpdate"));
      }

      setBackups((current) => upsertBackup(current, data.createBackup!));
      setPassword("");
      setConfirmPassword("");
      setCreateDialogOpen(false);
      setGlobalStatus(t("settings.backupsQueued"));
    } catch (error) {
      setGlobalStatus(mutationErrorMessage(error, t("status.failedToUpdate")));
    } finally {
      setCreatingRequest(false);
    }
  }, [canCreateBackup, client, password, setGlobalStatus, t]);

  const handleDeleteBackup = React.useCallback(async () => {
    if (!pendingDelete) {
      return;
    }

    setDeletingFilename(pendingDelete.filename);
    try {
      const { data, error } = await client
        .mutation<DeleteBackupMutationResult>(deleteBackupMutation, {
          filename: pendingDelete.filename,
        })
        .toPromise();
      if (error || data?.deleteBackup !== true) {
        throw error ?? new Error(t("status.failedToDelete"));
      }

      setBackups((current) =>
        current.filter((backup) => backup.filename !== pendingDelete.filename),
      );
      setPendingDelete(null);
      setGlobalStatus(t("settings.backupsDeleted"));
    } catch (error) {
      setGlobalStatus(mutationErrorMessage(error, t("status.failedToDelete")));
    } finally {
      setDeletingFilename(null);
    }
  }, [client, pendingDelete, setGlobalStatus, t]);

  const handleDownloadBackup = React.useCallback(async (backup: BackupInfoRecord) => {
    setDownloadingFilename(backup.filename);
    try {
      const { data, error } = await client
        .mutation<PrepareBackupDownloadMutationResult>(prepareBackupDownloadMutation, {
          filename: backup.filename,
        })
        .toPromise();
      const downloadUrl = data?.prepareBackupDownload?.downloadUrl;
      const downloadAuthorizationToken = data?.prepareBackupDownload?.downloadAuthorizationToken;
      if (error || !downloadUrl || !downloadAuthorizationToken) {
        throw error ?? new Error(t("status.failedToLoad"));
      }

      const response = await scryerFetch(buildAppUrl(downloadUrl), {
        headers: {
          Authorization: `Bearer ${downloadAuthorizationToken}`,
        },
      });
      if (!response.ok) {
        throw new Error(
          await readResponseErrorMessage(response, t("status.failedToLoad")),
        );
      }

      await saveDownloadResponse(response, backup.filename);
    } catch (error) {
      setGlobalStatus(mutationErrorMessage(error, t("status.failedToLoad")));
    } finally {
      setDownloadingFilename(null);
    }
  }, [client, setGlobalStatus, t]);

  const handleSaveAutoBackupSettings = React.useCallback(async () => {
    if (autoBackupKeyRequired) {
      setGlobalStatus(t("settings.autoBackupsKeyRequired"));
      return;
    }

    setAutoBackupSaving(true);
    try {
      const nextAutoBackupKey =
        clearAutoBackupKey ? null : autoBackupKey.trim().length > 0 ? autoBackupKey : null;
      const { data, error } = await client
        .mutation<UpdateAutoBackupSettingsMutationResult>(updateAutoBackupSettingsMutation, {
          input: {
            enabled: autoBackupSettings.enabled,
            dailyTimeLocal: autoBackupSettings.dailyTimeLocal,
            setAutoBackupKey: nextAutoBackupKey,
            clearAutoBackupKey,
          },
        })
        .toPromise();
      if (error || !data?.updateAutoBackupSettings) {
        throw error ?? new Error(t("status.failedToUpdate"));
      }

      setSavedAutoBackupSettings(data.updateAutoBackupSettings);
      setAutoBackupSettings(data.updateAutoBackupSettings);
      setAutoBackupKey("");
      setClearAutoBackupKey(false);
      setShowAutoBackupKey(false);
      setGlobalStatus(t("settings.autoBackupsSaved"));
    } catch (error) {
      setGlobalStatus(mutationErrorMessage(error, t("status.failedToUpdate")));
    } finally {
      setAutoBackupSaving(false);
    }
  }, [
    autoBackupKey,
    autoBackupKeyRequired,
    autoBackupSettings.dailyTimeLocal,
    autoBackupSettings.enabled,
    clearAutoBackupKey,
    client,
    setGlobalStatus,
    t,
  ]);

  if (pageLoading) {
    return (
      <div className="flex items-center gap-2 text-sm text-muted-foreground">
        <Loader2 className="h-4 w-4 animate-spin" />
        {t("label.loading")}
      </div>
    );
  }

  return (
    <>
      <div className="space-y-6 text-sm">
        <Card>
          <CardHeader className="flex flex-col gap-4 sm:flex-row sm:items-start sm:justify-between">
            <button
              type="button"
              className="flex min-w-0 flex-1 items-start gap-3 text-left"
              onClick={() => setAutoBackupExpanded((current) => !current)}
              aria-expanded={autoBackupExpanded}
            >
              <div className="min-w-0 flex-1 space-y-1">
                <CardTitle>{t("settings.autoBackupsTitle")}</CardTitle>
                <p className="text-sm text-muted-foreground">
                  {t("settings.autoBackupsDescription")}
                </p>
              </div>
            </button>
            <div className="flex shrink-0 justify-end sm:pt-1">
              <SettingsToggleSwitch
                checked={autoBackupSettings.enabled}
                disabled={autoBackupSaving}
                size="lg"
                ariaLabel={
                  autoBackupSettings.enabled
                    ? t("label.enabled")
                    : t("label.disabled")
                }
                onChange={(nextValue) =>
                  setAutoBackupSettings((current) => ({
                    ...current,
                    enabled: nextValue,
                  }))
                }
              />
            </div>
          </CardHeader>
          {autoBackupExpanded ? (
            <CardContent className="space-y-4">
              <div className="grid gap-4 md:grid-cols-2 md:items-stretch">
                <div className="flex h-full min-h-32 flex-col rounded-lg border border-border bg-muted/20 p-4">
                  <Label htmlFor="auto-backups-time" className="text-sm font-medium">
                    {t("settings.autoBackupsTime")}
                  </Label>
                  <div className="mt-3">
                    <TimePicker
                      id="auto-backups-time"
                      value={autoBackupSettings.dailyTimeLocal}
                      disabled={autoBackupSaving}
                      hourLabel={t("settings.autoBackupsHour")}
                      minuteLabel={t("settings.autoBackupsMinute")}
                      onChange={(nextValue) =>
                        setAutoBackupSettings((current) => ({
                          ...current,
                          dailyTimeLocal: nextValue,
                        }))
                      }
                    />
                  </div>
                  <p className="mt-auto pt-4 text-xs text-muted-foreground">
                    {t("settings.autoBackupsTimeHelp")}
                  </p>
                </div>

                <div className="flex h-full min-h-32 flex-col rounded-lg border border-border bg-muted/20 p-4">
                  <p className="text-sm font-medium">{t("settings.autoBackupsNextRun")}</p>
                  <p className="mt-3 text-base font-medium text-foreground">{autoBackupNextRunLabel}</p>
                  <p className="mt-auto pt-4 text-xs text-muted-foreground">
                    {t("settings.autoBackupsNextRunHelp")}
                  </p>
                </div>
              </div>

              <div className="space-y-2 text-sm">
                <div className="flex items-center gap-2">
                  <span className="font-medium">{t("settings.autoBackupsKeyLabel")}</span>
                  <InfoHelp
                    text={t("settings.autoBackupsKeyHelp")}
                    ariaLabel={t("settings.autoBackupsKeyHelpLabel")}
                  />
                </div>
                <div className="flex flex-col gap-3 sm:flex-row sm:items-center">
                  <div className="w-full max-w-sm space-y-2">
                    <div className="relative">
                      <Input
                        type={showAutoBackupKey ? "text" : "password"}
                        value={autoBackupKey}
                        placeholder={autoBackupKeyPlaceholder}
                        className="pr-11"
                        disabled={autoBackupSaving || clearAutoBackupKey}
                        onChange={(event) => {
                          const nextValue = event.target.value;
                          setAutoBackupKey(nextValue);
                          if (nextValue.trim().length > 0) {
                            setClearAutoBackupKey(false);
                          }
                        }}
                      />
                      <button
                        type="button"
                        className="absolute inset-y-0 right-0 flex w-10 items-center justify-center text-muted-foreground transition hover:text-foreground disabled:cursor-not-allowed disabled:opacity-50"
                        aria-label={
                          showAutoBackupKey
                            ? t("settings.autoBackupsHideKey")
                            : t("settings.autoBackupsShowKey")
                        }
                        disabled={autoBackupSaving || clearAutoBackupKey}
                        onClick={() => setShowAutoBackupKey((current) => !current)}
                      >
                        {showAutoBackupKey ? (
                          <EyeOff className="h-4 w-4" />
                        ) : (
                          <Eye className="h-4 w-4" />
                        )}
                      </button>
                    </div>
                    {autoBackupKeyRequired ? (
                      <p className="text-xs text-destructive">
                        {t("settings.autoBackupsKeyRequired")}
                      </p>
                    ) : null}
                  </div>

                  <div className="flex shrink-0 flex-wrap items-center gap-3 sm:min-w-64">
                    <Button
                      type="button"
                      onClick={() => void handleSaveAutoBackupSettings()}
                      disabled={!canSaveAutoBackupSettings}
                    >
                      {autoBackupSaving ? <Loader2 className="h-4 w-4 animate-spin" /> : null}
                      {t("label.save")}
                    </Button>

                    {autoBackupSettings.autoBackupKeyPresent ? (
                      <div className="flex min-w-0 items-center gap-3 rounded-lg border border-border bg-muted/20 px-4 py-3 sm:max-w-xl">
                        <Checkbox
                          id="auto-backups-clear-key"
                          className="size-5 rounded-md data-[state=checked]:border-rose-500 data-[state=checked]:bg-rose-500 data-[state=indeterminate]:border-rose-500 data-[state=indeterminate]:bg-rose-500"
                          checked={clearAutoBackupKey}
                          disabled={
                            autoBackupSaving || autoBackupSettings.enabled || autoBackupKey.length > 0
                          }
                          onCheckedChange={(checked) => {
                            const shouldClear = checked === true;
                            setClearAutoBackupKey(shouldClear);
                            if (shouldClear) {
                              setAutoBackupKey("");
                              setShowAutoBackupKey(false);
                            }
                          }}
                        />
                        <div className="flex min-w-0 items-center gap-2">
                          <Label htmlFor="auto-backups-clear-key" className="truncate">
                            {t("settings.autoBackupsClearKey")}
                          </Label>
                          <InfoHelp
                            text={t("settings.autoBackupsClearKeyHelp")}
                            ariaLabel={t("settings.autoBackupsClearKey")}
                          />
                        </div>
                      </div>
                    ) : null}
                  </div>
                </div>
              </div>
            </CardContent>
          ) : null}
        </Card>

        <div className="flex flex-col gap-4 md:flex-row md:items-start md:justify-between">
          <div className="space-y-1">
            <p className="text-muted-foreground">{t("settings.backupsSection")}</p>
          </div>
          <Button
            type="button"
            className="shrink-0"
            onClick={() => setCreateDialogOpen(true)}
            disabled={hasCreatingManualBackup}
          >
            <Plus className="h-4 w-4" />
            {t("settings.backupsCreate")}
          </Button>
        </div>

        {backups.length === 0 ? (
          <div className="rounded-lg border border-dashed border-border px-4 py-8 text-center text-sm text-muted-foreground">
            {t("settings.backupsEmpty")}
          </div>
        ) : (
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Bundle</TableHead>
                <TableHead>Created</TableHead>
                <TableHead>{t("label.status")}</TableHead>
                <TableHead>Size</TableHead>
                <TableHead className="text-right">{t("label.actions")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {backups.map((backup) => {
                const isDeleting = deletingFilename === backup.filename;
                const isDownloading = downloadingFilename === backup.filename;
                const disableActions = backup.status === "creating" || isDeleting;
                const statusLabel =
                  backup.status === "creating"
                    ? t("settings.backupsCreating")
                    : backup.status === "invalid"
                      ? t("settings.backupsInvalid")
                    : backup.status === "failed"
                      ? t("settings.backupsFailed")
                      : t("settings.backupsReady");

                return (
                  <TableRow key={backup.filename}>
                    <TableCell className="align-top">
                      <div className="space-y-1">
                        <div className="font-medium">{backup.filename}</div>
                        <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
                          <span className="rounded-full border border-border px-2 py-0.5">
                            {backup.encrypted
                              ? t("settings.backupsEncrypted")
                              : t("settings.backupsPlaintext")}
                          </span>
                          <span className="rounded-full border border-border px-2 py-0.5">
                            {autoBackupTriggerLabel(backup.trigger)}
                          </span>
                          <span>{backup.formatVersion}</span>
                          <span>{backup.sourceEngine}</span>
                          {backup.sourceMigrationKey ? <span>{backup.sourceMigrationKey}</span> : null}
                        </div>
                        {backup.errorMessage ? (
                          <p className="text-xs text-destructive">{backup.errorMessage}</p>
                        ) : null}
                      </div>
                    </TableCell>
                    <TableCell className="whitespace-nowrap text-muted-foreground">
                      {formatDateTime(backup.createdAt)}
                    </TableCell>
                    <TableCell>
                      <BackupStatusBadge status={backup.status} label={statusLabel} />
                    </TableCell>
                    <TableCell className="align-middle text-xs text-muted-foreground">
                      {formatBytes(backup.sizeBytes)}
                    </TableCell>
                    <TableCell>
                      <div className="flex items-center justify-end gap-1">
                        {backup.status === "ready" ? (
                          <Button
                            type="button"
                            variant="secondary"
                            size="icon-sm"
                            className={cn(
                              boxedActionButtonBaseClass,
                              boxedActionButtonToneClass.install,
                            )}
                            disabled={isDownloading || isDeleting}
                            onClick={() => void handleDownloadBackup(backup)}
                            title={t("settings.backupsDownload")}
                            aria-label={t("settings.backupsDownload")}
                          >
                            {isDownloading ? (
                              <Loader2 className="h-4 w-4 animate-spin" />
                            ) : (
                              <Download className="h-4 w-4" />
                            )}
                          </Button>
                        ) : null}
                        <Button
                          type="button"
                          variant="secondary"
                          size="icon-sm"
                          className={cn(
                            boxedActionButtonBaseClass,
                            boxedActionButtonToneClass.delete,
                          )}
                          disabled={disableActions}
                          onClick={() => setPendingDelete(backup)}
                          title={t("settings.backupsDelete")}
                          aria-label={t("settings.backupsDelete")}
                        >
                          {isDeleting ? (
                            <Loader2 className="h-4 w-4 animate-spin" />
                          ) : (
                            <Trash2 className="h-4 w-4" />
                          )}
                        </Button>
                      </div>
                    </TableCell>
                  </TableRow>
                );
              })}
            </TableBody>
          </Table>
        )}
      </div>

      <Dialog
        open={createDialogOpen}
        onOpenChange={(open) => {
          setCreateDialogOpen(open);
          if (!open) {
            setPassword("");
            setConfirmPassword("");
          }
        }}
      >
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>{t("settings.backupsCreateTitle")}</DialogTitle>
            <DialogDescription>{t("settings.backupsCreateDescription")}</DialogDescription>
          </DialogHeader>
          <div className="space-y-3">
            <label className="block space-y-2 text-sm">
              <span className="font-medium">{t("settings.password")}</span>
              <Input
                type="password"
                value={password}
                onChange={(event) => {
                  const nextPassword = event.target.value;
                  setPassword(nextPassword);
                  if (nextPassword.length === 0) {
                    setConfirmPassword("");
                  }
                }}
                placeholder={t("settings.password")}
                disabled={creatingRequest}
                required
              />
            </label>
            <label className="block space-y-2 text-sm">
              <span className="font-medium">{t("settings.backupsConfirmPassword")}</span>
              <Input
                type="password"
                value={confirmPassword}
                onChange={(event) => setConfirmPassword(event.target.value)}
                placeholder={t("settings.backupsConfirmPassword")}
                disabled={creatingRequest}
                required
              />
              {passwordMismatch ? (
                <p className="text-xs text-destructive">
                  {t("settings.backupsPasswordMismatch")}
                </p>
              ) : null}
            </label>
            <div className="rounded-lg border border-border bg-muted/30 p-3 text-xs text-muted-foreground">
              <div className="mb-1 flex items-center gap-2 text-foreground">
                <LockKeyhole className="h-3.5 w-3.5" />
                <span>{t("settings.backupsRequiredPassword")}</span>
              </div>
              <p>{t("settings.backupsPasswordHelp")}</p>
            </div>
          </div>
          <DialogFooter>
            <Button
              type="button"
              variant="secondary"
              onClick={() => setCreateDialogOpen(false)}
              disabled={creatingRequest}
            >
              {t("label.cancel")}
            </Button>
            <Button
              type="button"
              onClick={() => void handleCreateBackup()}
              disabled={!canCreateBackup}
            >
              {creatingRequest ? <Loader2 className="h-4 w-4 animate-spin" /> : null}
              {t("settings.backupsCreate")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <ConfirmDialog
        open={pendingDelete !== null}
        title={t("settings.backupsDelete")}
        description={t("settings.backupsDeleteConfirm")}
        confirmLabel={t("settings.backupsDelete")}
        cancelLabel={t("label.cancel")}
        isBusy={deletingFilename !== null}
        onConfirm={handleDeleteBackup}
        onCancel={() => {
          if (deletingFilename !== null) {
            return;
          }
          setPendingDelete(null);
        }}
      />
    </>
  );
}
