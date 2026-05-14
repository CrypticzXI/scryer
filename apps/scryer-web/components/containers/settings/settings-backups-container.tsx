import * as React from "react";
import { Download, Loader2, LockKeyhole, Plus, Trash2 } from "lucide-react";
import { useClient } from "urql";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import { createBackupMutation, deleteBackupMutation } from "@/lib/graphql/mutations";
import { backupsQuery } from "@/lib/graphql/queries";
import { scryerFetch } from "@/lib/graphql/urql-client";
import { getAuthToken } from "@/lib/hooks/use-auth";
import { getRuntimeBasePath } from "@/lib/runtime-config";

type BackupRowCount = {
  table: string;
  rowCount: string;
};

type BackupInfoRecord = {
  filename: string;
  sizeBytes: string;
  createdAt: string;
  formatVersion: string;
  sourceEngine: string;
  sourceMigrationKey: string | null;
  encrypted: boolean;
  rowCounts: BackupRowCount[];
  status: "creating" | "ready" | "failed";
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

function createAuthHeaders(init?: HeadersInit): Headers {
  const headers = new Headers(init);
  const token = getAuthToken();
  if (token) {
    headers.set("authorization", `Bearer ${token}`);
  }
  return headers;
}

async function readResponseErrorMessage(response: Response, fallback: string): Promise<string> {
  const contentType = response.headers.get("content-type") ?? "";
  if (contentType.includes("application/json")) {
    const payload = await response.json().catch(() => null) as { error?: string } | null;
    if (payload?.error?.trim()) {
      return payload.error.trim();
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

function totalRows(backup: BackupInfoRecord): number {
  return backup.rowCounts.reduce((sum, rowCount) => sum + Number(rowCount.rowCount || 0), 0);
}

function statusTone(status: BackupInfoRecord["status"]): string {
  switch (status) {
    case "creating":
      return "border-amber-500/30 bg-amber-500/10 text-amber-300";
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
  const [createDialogOpen, setCreateDialogOpen] = React.useState(false);
  const [password, setPassword] = React.useState("");
  const [creatingRequest, setCreatingRequest] = React.useState(false);
  const [pendingDelete, setPendingDelete] = React.useState<BackupInfoRecord | null>(null);
  const [deletingFilename, setDeletingFilename] = React.useState<string | null>(null);
  const [downloadingFilename, setDownloadingFilename] = React.useState<string | null>(null);
  const hasCreatingBackup = backups.some((backup) => backup.status === "creating");

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

  React.useEffect(() => {
    void fetchBackups();
  }, [fetchBackups]);

  React.useEffect(() => {
    if (!backups.some((backup) => backup.status === "creating")) {
      return undefined;
    }

    const timeoutId = window.setTimeout(() => {
      void fetchBackups();
    }, 2500);
    return () => window.clearTimeout(timeoutId);
  }, [backups, fetchBackups]);

  const handleCreateBackup = React.useCallback(async () => {
    setCreatingRequest(true);
    try {
      const nextPassword = password;
      const { data, error } = await client
        .mutation<CreateBackupMutationResult>(createBackupMutation, {
          password: nextPassword.length > 0 ? nextPassword : null,
        })
        .toPromise();
      if (error || !data?.createBackup) {
        throw error ?? new Error(t("status.failedToUpdate"));
      }

      setBackups((current) => upsertBackup(current, data.createBackup!));
      setPassword("");
      setCreateDialogOpen(false);
      setGlobalStatus(t("settings.backupsQueued"));
    } catch (error) {
      setGlobalStatus(mutationErrorMessage(error, t("status.failedToUpdate")));
    } finally {
      setCreatingRequest(false);
    }
  }, [client, password, setGlobalStatus, t]);

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
      const response = await scryerFetch(
        buildAppUrl(`/admin/backups/${encodeURIComponent(backup.filename)}/download`),
        {
          headers: createAuthHeaders(),
        },
      );
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
  }, [setGlobalStatus, t]);

  if (loading) {
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
        <div className="flex flex-col gap-4 md:flex-row md:items-start md:justify-between">
          <div className="space-y-1">
            <p className="text-muted-foreground">{t("settings.backupsSection")}</p>
          </div>
          <Button
            type="button"
            className="shrink-0"
            onClick={() => setCreateDialogOpen(true)}
            disabled={hasCreatingBackup}
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
                <TableHead>Summary</TableHead>
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
                    <TableCell className="align-top">
                      <div className="space-y-1 text-xs text-muted-foreground">
                        <div>{formatBytes(backup.sizeBytes)}</div>
                        <div>
                          {t("settings.backupsTables", {
                            count: backup.rowCounts.length.toLocaleString(),
                          })}
                        </div>
                        <div>
                          {t("settings.backupsRows", {
                            count: totalRows(backup).toLocaleString(),
                          })}
                        </div>
                      </div>
                    </TableCell>
                    <TableCell>
                      <div className="flex items-center justify-end gap-2">
                        {backup.status === "ready" ? (
                          <Button
                            type="button"
                            variant="outline"
                            size="sm"
                            disabled={isDownloading || isDeleting}
                            onClick={() => void handleDownloadBackup(backup)}
                          >
                            {isDownloading ? (
                              <Loader2 className="h-4 w-4 animate-spin" />
                            ) : (
                              <Download className="h-4 w-4" />
                            )}
                            {t("settings.backupsDownload")}
                          </Button>
                        ) : null}
                        <Button
                          type="button"
                          variant="outline"
                          size="sm"
                          disabled={disableActions}
                          onClick={() => setPendingDelete(backup)}
                        >
                          {isDeleting ? (
                            <Loader2 className="h-4 w-4 animate-spin" />
                          ) : (
                            <Trash2 className="h-4 w-4" />
                          )}
                          {t("settings.backupsDelete")}
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
                onChange={(event) => setPassword(event.target.value)}
                placeholder={t("settings.password")}
                disabled={creatingRequest}
              />
            </label>
            <div className="rounded-lg border border-border bg-muted/30 p-3 text-xs text-muted-foreground">
              <div className="mb-1 flex items-center gap-2 text-foreground">
                <LockKeyhole className="h-3.5 w-3.5" />
                <span>{t("settings.backupsOptionalPassword")}</span>
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
              disabled={creatingRequest || hasCreatingBackup}
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
