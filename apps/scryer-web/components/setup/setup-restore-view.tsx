import * as React from "react";
import { ArrowLeft, Loader2, LockKeyhole, RotateCcw, Upload } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { scryerFetch } from "@/lib/graphql/urql-client";
import { getAuthToken } from "@/lib/hooks/use-auth";
import { getRuntimeBasePath, getRuntimeGraphqlUrl } from "@/lib/runtime-config";
import { cn } from "@/lib/utils";

const INSPECT_RESTORE_BUNDLE_OPERATION = `
  mutation InspectRestoreBundle($bundleUpload: Upload!, $password: String) {
    inspectRestoreBundle(bundleUpload: $bundleUpload, password: $password) {
      uploadId
      summary {
        formatVersion
        createdAt
        sourceScryerVersion
        sourceEngine
        sourceMigrationKey
        encrypted
        rowCounts {
          table
          rowCount
        }
        totalRows
      }
    }
  }
`;

const APPLY_RESTORE_BUNDLE_OPERATION = `
  mutation ApplyRestoreBundle($uploadId: String!, $password: String) {
    applyRestoreBundle(uploadId: $uploadId, password: $password) {
      summary {
        formatVersion
      }
    }
  }
`;

type RestoreSummaryPayload = {
  formatVersion: string;
  createdAt: string;
  sourceScryerVersion: string;
  sourceEngine: string;
  sourceMigrationKey: string | null;
  encrypted: boolean;
  rowCounts: Array<{
    table: string;
    rowCount: string;
  }>;
  totalRows: string;
};

type RestoreInspectResponse = {
  data?: {
    inspectRestoreBundle?: {
      uploadId: string;
      summary: RestoreSummaryPayload;
    } | null;
  };
  errors?: Array<{
    message: string;
  }>;
};

type RestoreApplyResponse = {
  data?: {
    applyRestoreBundle?: {
      summary: {
        formatVersion: string;
      };
    } | null;
  };
  errors?: Array<{
    message: string;
  }>;
};

interface SetupRestoreViewProps {
  t: (
    key: string,
    values?: Record<string, string | number | boolean | null | undefined>,
  ) => string;
  onBack: () => void;
  onBackendRestarting: () => void;
}

function buildFrontendUrl(path: string): string {
  const basePath = getRuntimeBasePath();
  if (basePath === "/") {
    return path;
  }
  return path === "/" ? basePath : `${basePath}${path}`;
}

function graphqlUrl(): string {
  return new URL(getRuntimeGraphqlUrl(), window.location.origin).toString();
}

function createAuthHeaders(init?: HeadersInit): Headers {
  const headers = new Headers(init);
  const token = getAuthToken();
  if (token) {
    headers.set("authorization", `Bearer ${token}`);
  }
  return headers;
}

function graphqlErrorMessage(
  payload: { errors?: Array<{ message: string }> } | undefined,
): string | null {
  return payload?.errors?.find((entry) => entry.message.trim())?.message.trim() ?? null;
}

async function parseJsonResponse<T>(response: Response): Promise<T | undefined> {
  const text = await response.text().catch(() => "");
  if (!text.trim()) {
    return undefined;
  }

  return JSON.parse(text) as T;
}

function formatDateTime(value: string): string {
  const timestamp = Date.parse(value);
  if (!Number.isFinite(timestamp)) {
    return value;
  }
  return new Date(timestamp).toLocaleString();
}

export function SetupRestoreView({
  t,
  onBack,
  onBackendRestarting,
}: SetupRestoreViewProps) {
  const [selectedBundle, setSelectedBundle] = React.useState<File | null>(null);
  const [password, setPassword] = React.useState("");
  const [fileInputKey, setFileInputKey] = React.useState(0);
  const [inspecting, setInspecting] = React.useState(false);
  const [applying, setApplying] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [uploadId, setUploadId] = React.useState<string | null>(null);
  const [summary, setSummary] = React.useState<RestoreSummaryPayload | null>(null);
  const [isDragActive, setIsDragActive] = React.useState(false);
  const fileInputRef = React.useRef<HTMLInputElement | null>(null);

  const bundleLooksEncrypted
    = selectedBundle?.name.toLowerCase().endsWith(".enc") ?? false;
  const requiresPassword = summary?.encrypted ?? bundleLooksEncrypted;
  const rowCounts = summary
    ? [...summary.rowCounts].sort((left, right) => left.table.localeCompare(right.table))
    : [];

  const handleBundleSelected = React.useCallback((file: File | null) => {
    setError(null);
    setSelectedBundle(file);
    setUploadId(null);
    setSummary(null);
    if (!file?.name.toLowerCase().endsWith(".enc")) {
      setPassword("");
    }
  }, []);

  const resetSelection = React.useCallback(() => {
    setSelectedBundle(null);
    setPassword("");
    setUploadId(null);
    setSummary(null);
    setError(null);
    setIsDragActive(false);
    setFileInputKey((current) => current + 1);
  }, []);

  const handleInspect = React.useCallback(async () => {
    if (!selectedBundle) {
      setError(t("setup.restoreNoFile"));
      return;
    }
    if (requiresPassword && password.length === 0) {
      setError(t("setup.restorePasswordRequired"));
      return;
    }

    setInspecting(true);
    setError(null);
    try {
      const formData = new FormData();
      formData.append(
        "operations",
        JSON.stringify({
          query: INSPECT_RESTORE_BUNDLE_OPERATION,
          variables: {
            bundleUpload: null,
            password: password.length > 0 ? password : null,
          },
        }),
      );
      formData.append("map", JSON.stringify({ "0": ["variables.bundleUpload"] }));
      formData.append("0", selectedBundle, selectedBundle.name);

      const response = await scryerFetch(graphqlUrl(), {
        method: "POST",
        headers: createAuthHeaders(),
        body: formData,
      });
      const payload = await parseJsonResponse<RestoreInspectResponse>(response);
      if (!response.ok) {
        throw new Error(
          graphqlErrorMessage(payload) ?? t("status.failedToLoad"),
        );
      }
      const result = payload?.data?.inspectRestoreBundle;
      if (!result) {
        throw new Error(graphqlErrorMessage(payload) ?? t("status.failedToLoad"));
      }

      setUploadId(result.uploadId);
      setSummary(result.summary);
    } catch (nextError) {
      setError(
        nextError instanceof Error && nextError.message.trim()
          ? nextError.message.trim()
          : t("status.failedToLoad"),
      );
    } finally {
      setInspecting(false);
    }
  }, [password, requiresPassword, selectedBundle, t]);

  const handleApply = React.useCallback(async () => {
    if (!uploadId || !summary) {
      return;
    }

    setApplying(true);
    setError(null);
    try {
      const response = await scryerFetch(graphqlUrl(), {
        method: "POST",
        headers: createAuthHeaders({
          "content-type": "application/json",
        }),
        body: JSON.stringify({
          query: APPLY_RESTORE_BUNDLE_OPERATION,
          variables: {
            uploadId,
            password: summary.encrypted ? password : null,
          },
        }),
      });
      const payload = await parseJsonResponse<RestoreApplyResponse>(response);
      if (!response.ok) {
        throw new Error(
          graphqlErrorMessage(payload) ?? t("status.failedToUpdate"),
        );
      }
      if (!payload?.data?.applyRestoreBundle) {
        throw new Error(graphqlErrorMessage(payload) ?? t("status.failedToUpdate"));
      }

      window.history.replaceState(null, "", buildFrontendUrl("/"));
      onBackendRestarting();
    } catch (nextError) {
      setApplying(false);
      setError(
        nextError instanceof Error && nextError.message.trim()
          ? nextError.message.trim()
          : t("status.failedToUpdate"),
      );
    }
  }, [onBackendRestarting, password, summary, t, uploadId]);

  return (
    <div className="w-full space-y-6">
      <div className="text-center">
        <h2 className="mb-2 text-xl font-semibold">{t("setup.restoreTitle")}</h2>
        <p className="text-sm text-muted-foreground">{t("setup.restoreDescription")}</p>
      </div>

      {!summary ? (
        <Card>
          <CardContent className="space-y-5 p-5">
            <div className="space-y-2 text-sm">
              <span className="font-medium">{t("setup.restoreSelectBundle")}</span>
              <input
                id="setup-restore-file-input"
                key={fileInputKey}
                ref={fileInputRef}
                type="file"
                accept=".tar.zst,.enc,.zst"
                className="hidden"
                onChange={(event) => {
                  handleBundleSelected(event.target.files?.[0] ?? null);
                }}
                disabled={inspecting}
              />
              <div
                onDragOver={(event) => {
                  event.preventDefault();
                  event.dataTransfer.dropEffect = "copy";
                  setIsDragActive(true);
                }}
                onDragLeave={(event) => {
                  event.preventDefault();
                  if (event.currentTarget.contains(event.relatedTarget as Node | null)) {
                    return;
                  }
                  setIsDragActive(false);
                }}
                onDrop={(event) => {
                  event.preventDefault();
                  setIsDragActive(false);
                  handleBundleSelected(event.dataTransfer.files?.[0] ?? null);
                }}
                className={cn(
                  "rounded-2xl border border-dashed px-5 py-8 transition-colors",
                  isDragActive
                    ? "border-primary bg-primary/8"
                    : "border-border/80 bg-muted/20 hover:border-primary/50 hover:bg-muted/30",
                )}
              >
                <div className="mx-auto flex max-w-xl flex-col items-center gap-4 text-center">
                  <div className="flex h-12 w-12 items-center justify-center rounded-full bg-primary/10 text-primary">
                    <Upload className="h-5 w-5" />
                  </div>
                  <div className="space-y-1">
                    <p className="break-all text-base font-medium text-foreground">
                      {selectedBundle
                        ? selectedBundle.name
                        : t("setup.restoreDropTargetTitle")}
                    </p>
                    <p className="text-sm text-muted-foreground">
                      {selectedBundle
                        ? bundleLooksEncrypted
                          ? t("setup.restoreDropTargetEncryptedSelected")
                          : t("setup.restoreDropTargetSelected")
                        : t("setup.restoreDropTargetDescription")}
                    </p>
                  </div>
                  <div className="flex flex-wrap items-center justify-center gap-3">
                    <Button
                      id="setup-restore-select-file"
                      type="button"
                      onClick={() => fileInputRef.current?.click()}
                      disabled={inspecting}
                    >
                      {t("setup.restoreSelectFile")}
                    </Button>
                    {selectedBundle ? (
                      <Button
                        id="setup-restore-clear-file"
                        type="button"
                        variant="destructive"
                        onClick={resetSelection}
                        disabled={inspecting}
                      >
                        {t("setup.restoreClearFile")}
                      </Button>
                    ) : null}
                  </div>
                  <p className="text-xs text-muted-foreground">
                    {t("setup.restoreDropTargetFormats")}
                  </p>
                </div>
              </div>
            </div>

            {requiresPassword ? (
              <label className="block space-y-2 text-sm">
                <span className="font-medium">{t("settings.password")}</span>
                <Input
                  id="setup-restore-password"
                  type="password"
                  value={password}
                  onChange={(event) => {
                    setError(null);
                    setPassword(event.target.value);
                  }}
                  placeholder={t("settings.password")}
                  disabled={inspecting}
                />
              </label>
            ) : null}

            {requiresPassword ? (
              <div className="rounded-lg border border-border bg-muted/30 p-3 text-xs text-muted-foreground">
                <div className="mb-1 flex items-center gap-2 text-foreground">
                  <LockKeyhole className="h-3.5 w-3.5" />
                  <span>{t("setup.restorePasswordHelp")}</span>
                </div>
              </div>
            ) : null}

            {error ? (
              <p id="setup-restore-error" className="text-sm text-destructive">{error}</p>
            ) : null}
          </CardContent>
        </Card>
      ) : (
        <div className="space-y-4">
          <Card>
            <CardContent className="space-y-5 p-5">
              <div className="flex flex-wrap items-center gap-2">
                <span className="rounded-full border border-border px-2 py-0.5 text-xs font-medium">
                  {summary.encrypted
                    ? t("settings.backupsEncrypted")
                    : t("settings.backupsPlaintext")}
                </span>
                <span className="rounded-full border border-border px-2 py-0.5 text-xs font-medium">
                  {summary.formatVersion}
                </span>
              </div>

              <div className="grid gap-3 text-sm sm:grid-cols-2">
                <div>
                  <p className="text-xs uppercase tracking-wide text-muted-foreground">
                    {t("setup.restoreCreatedAt")}
                  </p>
                  <p>{formatDateTime(summary.createdAt)}</p>
                </div>
                <div>
                  <p className="text-xs uppercase tracking-wide text-muted-foreground">
                    {t("setup.restoreSourceVersion")}
                  </p>
                  <p>{summary.sourceScryerVersion}</p>
                </div>
                <div>
                  <p className="text-xs uppercase tracking-wide text-muted-foreground">
                    {t("setup.restoreSourceEngine")}
                  </p>
                  <p>{summary.sourceEngine}</p>
                </div>
                <div>
                  <p className="text-xs uppercase tracking-wide text-muted-foreground">
                    {t("setup.restoreMigrationKey")}
                  </p>
                  <p>{summary.sourceMigrationKey ?? "-"}</p>
                </div>
                <div>
                  <p className="text-xs uppercase tracking-wide text-muted-foreground">
                    {t("setup.restoreTotalRows")}
                  </p>
                  <p>{Number(summary.totalRows).toLocaleString()}</p>
                </div>
                <div>
                  <p className="text-xs uppercase tracking-wide text-muted-foreground">
                    Tables
                  </p>
                  <p>{rowCounts.length.toLocaleString()}</p>
                </div>
              </div>

              <div className="space-y-2">
                <h3 className="text-sm font-semibold">{t("setup.restoreSummaryTitle")}</h3>
                <div className="max-h-64 overflow-y-auto rounded-lg border border-border">
                  <Table>
                    <TableHeader>
                      <TableRow>
                        <TableHead>Table</TableHead>
                        <TableHead className="text-right">Rows</TableHead>
                      </TableRow>
                    </TableHeader>
                    <TableBody>
                      {rowCounts.map((entry) => (
                        <TableRow key={entry.table}>
                          <TableCell className="font-mono text-xs">{entry.table}</TableCell>
                          <TableCell className="text-right text-xs text-muted-foreground">
                            {Number(entry.rowCount).toLocaleString()}
                          </TableCell>
                        </TableRow>
                      ))}
                    </TableBody>
                  </Table>
                </div>
              </div>

              <p className="text-sm text-muted-foreground">
                {t("setup.restoreConfirmDescription")}
              </p>

              {error ? (
                <p id="setup-restore-error" className="text-sm text-destructive">{error}</p>
              ) : null}
            </CardContent>
          </Card>
        </div>
      )}

      <div className="flex flex-wrap items-center justify-between gap-2">
        <Button id="setup-restore-back" variant="ghost" onClick={onBack} disabled={inspecting || applying}>
          <ArrowLeft className="mr-2 h-4 w-4" />
          {t("setup.back")}
        </Button>

        <div className="flex flex-wrap items-center gap-2">
          {summary ? (
            <Button
              id="setup-restore-choose-another"
              type="button"
              variant="outline"
              onClick={resetSelection}
              disabled={applying}
            >
              <RotateCcw className="h-4 w-4" />
              {t("setup.restoreChooseAnother")}
            </Button>
          ) : null}

          {summary ? (
            <Button id="setup-restore-apply" type="button" onClick={() => void handleApply()} disabled={applying}>
              {applying ? <Loader2 className="h-4 w-4 animate-spin" /> : <Upload className="h-4 w-4" />}
              {t("setup.restoreApply")}
            </Button>
          ) : (
            <Button
              id="setup-restore-inspect"
              type="button"
              onClick={() => void handleInspect()}
              disabled={
                inspecting
                || !selectedBundle
                || (requiresPassword && password.length === 0)
              }
            >
              {inspecting ? <Loader2 className="h-4 w-4 animate-spin" /> : <Upload className="h-4 w-4" />}
              {t("setup.restoreInspect")}
            </Button>
          )}
        </div>
      </div>
    </div>
  );
}
