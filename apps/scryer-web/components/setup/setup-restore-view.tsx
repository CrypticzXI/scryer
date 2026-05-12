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
import { getRuntimeBasePath } from "@/lib/runtime-config";

type RestoreSummaryPayload = {
  format_version: string;
  created_at: string;
  source_scryer_version: string;
  source_engine: string;
  source_migration_key: string | null;
  encrypted: boolean;
  row_counts: Record<string, number>;
  total_rows: number;
};

type RestoreInspectResponse = {
  upload_id: string;
  summary: RestoreSummaryPayload;
};

interface SetupRestoreViewProps {
  t: (
    key: string,
    values?: Record<string, string | number | boolean | null | undefined>,
  ) => string;
  onBack: () => void;
  onBackendRestarting: () => void;
}

function buildAppUrl(path: string): string {
  const basePath = getRuntimeBasePath();
  if (basePath === "/") {
    return path;
  }
  return path === "/" ? basePath : `${basePath}${path}`;
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

  const bundleLooksEncrypted = selectedBundle?.name.toLowerCase().endsWith(".age") ?? false;
  const requiresPassword = summary?.encrypted ?? bundleLooksEncrypted;
  const rowCounts = summary
    ? Object.entries(summary.row_counts).sort((left, right) => left[0].localeCompare(right[0]))
    : [];

  const resetSelection = React.useCallback(() => {
    setSelectedBundle(null);
    setPassword("");
    setUploadId(null);
    setSummary(null);
    setError(null);
    setFileInputKey((current) => current + 1);
  }, []);

  const handleInspect = React.useCallback(async () => {
    if (!selectedBundle) {
      setError(t("setup.restoreNoFile"));
      return;
    }
    if (requiresPassword && password.trim().length === 0) {
      setError(t("setup.restorePasswordRequired"));
      return;
    }

    setInspecting(true);
    setError(null);
    try {
      const formData = new FormData();
      formData.set("bundle", selectedBundle);
      if (password.trim()) {
        formData.set("password", password.trim());
      }

      const response = await scryerFetch(buildAppUrl("/setup/restore/inspect"), {
        method: "POST",
        headers: createAuthHeaders(),
        body: formData,
      });
      if (!response.ok) {
        throw new Error(
          await readResponseErrorMessage(response, t("status.failedToLoad")),
        );
      }

      const payload = await response.json() as RestoreInspectResponse;
      setUploadId(payload.upload_id);
      setSummary(payload.summary);
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
      const response = await scryerFetch(buildAppUrl("/setup/restore/apply"), {
        method: "POST",
        headers: createAuthHeaders({
          "content-type": "application/json",
        }),
        body: JSON.stringify({
          upload_id: uploadId,
          password: summary.encrypted ? password.trim() : undefined,
        }),
      });
      if (!response.ok) {
        throw new Error(
          await readResponseErrorMessage(response, t("status.failedToUpdate")),
        );
      }

      window.history.replaceState(null, "", buildAppUrl("/"));
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
            <label className="block space-y-2 text-sm">
              <span className="font-medium">{t("setup.restoreSelectBundle")}</span>
              <Input
                key={fileInputKey}
                type="file"
                accept=".scryer-backup.tar.zst,.scryer-backup.age,.age,.zst"
                onChange={(event) => {
                  setError(null);
                  setSelectedBundle(event.target.files?.[0] ?? null);
                }}
                disabled={inspecting}
              />
            </label>

            <label className="block space-y-2 text-sm">
              <span className="font-medium">{t("settings.password")}</span>
              <Input
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

            <div className="rounded-lg border border-border bg-muted/30 p-3 text-xs text-muted-foreground">
              <div className="mb-1 flex items-center gap-2 text-foreground">
                <LockKeyhole className="h-3.5 w-3.5" />
                <span>{t("setup.restorePasswordHelp")}</span>
              </div>
              {selectedBundle ? (
                <p className="break-all">
                  {selectedBundle.name}
                </p>
              ) : null}
            </div>

            {error ? (
              <p className="text-sm text-destructive">{error}</p>
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
                  {summary.format_version}
                </span>
              </div>

              <div className="grid gap-3 text-sm sm:grid-cols-2">
                <div>
                  <p className="text-xs uppercase tracking-wide text-muted-foreground">
                    {t("setup.restoreCreatedAt")}
                  </p>
                  <p>{formatDateTime(summary.created_at)}</p>
                </div>
                <div>
                  <p className="text-xs uppercase tracking-wide text-muted-foreground">
                    {t("setup.restoreSourceVersion")}
                  </p>
                  <p>{summary.source_scryer_version}</p>
                </div>
                <div>
                  <p className="text-xs uppercase tracking-wide text-muted-foreground">
                    {t("setup.restoreSourceEngine")}
                  </p>
                  <p>{summary.source_engine}</p>
                </div>
                <div>
                  <p className="text-xs uppercase tracking-wide text-muted-foreground">
                    {t("setup.restoreMigrationKey")}
                  </p>
                  <p>{summary.source_migration_key ?? "-"}</p>
                </div>
                <div>
                  <p className="text-xs uppercase tracking-wide text-muted-foreground">
                    {t("setup.restoreTotalRows")}
                  </p>
                  <p>{summary.total_rows.toLocaleString()}</p>
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
                      {rowCounts.map(([table, rowCount]) => (
                        <TableRow key={table}>
                          <TableCell className="font-mono text-xs">{table}</TableCell>
                          <TableCell className="text-right text-xs text-muted-foreground">
                            {rowCount.toLocaleString()}
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
                <p className="text-sm text-destructive">{error}</p>
              ) : null}
            </CardContent>
          </Card>
        </div>
      )}

      <div className="flex flex-wrap items-center justify-between gap-2">
        <Button variant="ghost" onClick={onBack} disabled={inspecting || applying}>
          <ArrowLeft className="mr-2 h-4 w-4" />
          {t("setup.back")}
        </Button>

        <div className="flex flex-wrap items-center gap-2">
          {summary ? (
            <Button
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
            <Button type="button" onClick={() => void handleApply()} disabled={applying}>
              {applying ? <Loader2 className="h-4 w-4 animate-spin" /> : <Upload className="h-4 w-4" />}
              {t("setup.restoreApply")}
            </Button>
          ) : (
            <Button
              type="button"
              onClick={() => void handleInspect()}
              disabled={
                inspecting
                || !selectedBundle
                || (requiresPassword && password.trim().length === 0)
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
