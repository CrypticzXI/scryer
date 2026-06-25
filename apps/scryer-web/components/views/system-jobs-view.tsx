import { ArrowDown, ArrowUp } from "lucide-react";
import { useCallback, useMemo, useState } from "react";

import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { useLibraryScanProgress } from "@/lib/context/library-scan-progress-context";
import { useTranslate } from "@/lib/context/translate-context";
import { useUiDateTimeFormat } from "@/lib/context/ui-settings-context";
import type { Facet, JobDefinition, JobKey, JobRun, LibraryScanStatus } from "@/lib/types";
import type { UiDateTimeFormat } from "@/lib/types/settings";
import { formatUiDate, formatUiDateTime, formatUiTime } from "@/lib/utils/date-format";
import { isTerminalJobRunStatus } from "@/lib/utils/job-runs";
import { cn } from "@/lib/utils";

type SystemJobsViewState = {
  jobs: JobDefinition[];
  activeRuns: JobRun[];
  recentRuns: JobRun[];
  selectedJobKey: JobKey | null;
  selectedJobHistory: JobRun[];
  jobHistoryLoading: boolean;
  triggeringKeys: Partial<Record<JobKey, boolean>>;
  onSelectJob: (jobKey: JobKey | null) => void;
  onTriggerJob: (jobKey: JobKey) => void;
};

type HealthCheckIssue = {
  source: string;
  status: string;
  message: string;
};

type SortKey = "name" | "nextRun" | "lastRun" | "status";
type SortDirection = "asc" | "desc";

type JobTableRow = {
  job: JobDefinition;
  activeRun: JobRun | null;
  activeLibraryScan: ReturnType<ReturnType<typeof useLibraryScanProgress>["getActiveSession"]> | null;
  lastRun: JobRun | null;
  status: JobRun["status"] | "idle";
  isDisabled: boolean;
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function formatDate(
  value: string | null | undefined,
  t: ReturnType<typeof useTranslate>,
  dateTimeFormat: UiDateTimeFormat,
): string {
  if (!value) {
    return t("jobs.never");
  }
  return formatUiDateTime(value, dateTimeFormat, { fallback: value });
}

function renderTableDateTime(
  value: string | null | undefined,
  t: ReturnType<typeof useTranslate>,
  dateTimeFormat: UiDateTimeFormat,
) {
  if (!value) {
    return t("jobs.never");
  }

  return (
    <div className="space-y-0.5 leading-tight">
      <div>{formatUiDate(value, dateTimeFormat, { fallback: value })}</div>
      <div>{formatUiTime(value, dateTimeFormat, { fallback: "" })}</div>
    </div>
  );
}

function runStatusTone(status: JobRun["status"] | "idle"): string {
  switch (status) {
    case "failed":
      return "text-red-400";
    case "warning":
      return "text-amber-400";
    case "completed":
      return "text-emerald-400";
    case "queued":
    case "discovering":
    case "running":
      return "text-sky-400";
    default:
      return "text-muted-foreground";
  }
}

function runStatusLabel(
  status: JobRun["status"] | "idle",
  t: ReturnType<typeof useTranslate>,
): string {
  switch (status) {
    case "idle":
      return t("jobs.status.idle");
    case "queued":
      return t("jobs.status.queued");
    case "discovering":
      return t("jobs.status.discovering");
    case "running":
      return t("jobs.status.running");
    case "completed":
      return t("jobs.status.completed");
    case "warning":
      return t("jobs.status.warning");
    case "failed":
      return t("jobs.status.failed");
  }
}

function parseHealthCheckIssues(run: JobRun): HealthCheckIssue[] {
  if (run.jobKey !== "health_checks" || !run.summaryJson) {
    return [];
  }

  try {
    const parsed = JSON.parse(run.summaryJson) as unknown;
    if (!isRecord(parsed) || !Array.isArray(parsed.checks)) {
      return [];
    }

    return parsed.checks.flatMap((check) => {
      if (
        !isRecord(check) ||
        typeof check.source !== "string" ||
        typeof check.status !== "string" ||
        typeof check.message !== "string" ||
        check.status === "ok"
      ) {
        return [];
      }

      return [
        {
          source: check.source,
          status: check.status,
          message: check.message,
        },
      ];
    });
  } catch {
    return [];
  }
}

function formatHealthCheckSource(source: string): string {
  const withSpaces = source.replace(/([a-z0-9])([A-Z])/g, "$1 $2").trim();
  return withSpaces.length > 0 ? withSpaces : source;
}

function healthCheckStatusTone(status: string): string {
  switch (status) {
    case "error":
      return "text-red-400";
    case "warning":
      return "text-amber-400";
    case "ok":
      return "text-emerald-400";
    default:
      return "text-muted-foreground";
  }
}

function formatHealthCheckStatus(status: string): string {
  if (!status) {
    return "Unknown";
  }
  return `${status.charAt(0).toUpperCase()}${status.slice(1)}`;
}

function triggerSourceLabel(
  triggerSource: JobRun["triggerSource"],
  t: ReturnType<typeof useTranslate>,
): string {
  switch (triggerSource) {
    case "manual":
      return t("jobs.triggerSource.manual");
    case "scheduled_startup":
      return t("jobs.triggerSource.scheduledStartup");
    case "scheduled_interval":
      return t("jobs.triggerSource.scheduledInterval");
    case "scheduled_daily":
      return t("jobs.triggerSource.scheduledDaily");
    case "system_internal":
      return t("jobs.triggerSource.systemInternal");
  }
}

function libraryFacetForJob(jobKey: JobKey): Facet | null {
  switch (jobKey) {
    case "library_scan_movies":
    case "background_library_refresh_movies":
      return "movie";
    case "library_scan_series":
    case "background_library_refresh_series":
      return "series";
    case "library_scan_anime":
    case "background_library_refresh_anime":
      return "anime";
    default:
      return null;
  }
}

function isRunButtonDisabled(
  hasActiveExecution: boolean,
  isTriggering: boolean,
): boolean {
  if (isTriggering) {
    return true;
  }

  if (hasActiveExecution) {
    return true;
  }

  return false;
}

function compareText(left: string, right: string): number {
  return left.localeCompare(right, undefined, { sensitivity: "base", numeric: true });
}

function compareMaybeDates(
  left: string | null | undefined,
  right: string | null | undefined,
): number {
  if (!left && !right) {
    return 0;
  }
  if (!left) {
    return 1;
  }
  if (!right) {
    return -1;
  }

  const leftTime = new Date(left).getTime();
  const rightTime = new Date(right).getTime();

  if (Number.isNaN(leftTime) && Number.isNaN(rightTime)) {
    return compareText(left, right);
  }
  if (Number.isNaN(leftTime)) {
    return 1;
  }
  if (Number.isNaN(rightTime)) {
    return -1;
  }

  return leftTime - rightTime;
}

function statusSortWeight(status: JobRun["status"] | "idle"): number {
  switch (status) {
    case "running":
    case "discovering":
    case "queued":
      return 0;
    case "failed":
      return 1;
    case "warning":
      return 2;
    case "completed":
      return 3;
    case "idle":
    default:
      return 4;
  }
}

function jobStatusFromLibraryScanStatus(
  status: LibraryScanStatus,
): JobRun["status"] {
  switch (status) {
    case "discovering":
      return "discovering";
    case "running":
      return "running";
    case "completed":
      return "completed";
    case "canceled":
    case "warning":
      return "warning";
    case "failed":
      return "failed";
  }
}

function isStaleActiveRun(
  activeRun: JobRun | null | undefined,
  lastRun: JobRun | null | undefined,
): boolean {
  if (!activeRun || !lastRun || !isTerminalJobRunStatus(lastRun.status)) {
    return false;
  }

  if (lastRun.id === activeRun.id) {
    return true;
  }

  return lastRun.startedAt.localeCompare(activeRun.startedAt) >= 0;
}

export function SystemJobsView({ state }: { state: SystemJobsViewState }) {
  const t = useTranslate();
  const dateTimeFormat = useUiDateTimeFormat();
  const { getActiveSession } = useLibraryScanProgress();
  const [sortKey, setSortKey] = useState<SortKey>("name");
  const [sortDirection, setSortDirection] = useState<SortDirection>("asc");
  const {
    jobs,
    activeRuns,
    recentRuns,
    selectedJobKey,
    selectedJobHistory,
    jobHistoryLoading,
    triggeringKeys,
    onSelectJob,
    onTriggerJob,
  } = state;

  const selectedJob = useMemo(
    () => jobs.find((job) => job.key === selectedJobKey) ?? null,
    [jobs, selectedJobKey],
  );

  const activeRunsByJob = useMemo(
    () => Object.fromEntries(activeRuns.map((run) => [run.jobKey, run])),
    [activeRuns],
  );

  const lastRunsByJob = useMemo(() => {
    const map = new Map<JobKey, JobRun>();
    for (const run of recentRuns) {
      if (!map.has(run.jobKey)) {
        map.set(run.jobKey, run);
      }
    }
    return map;
  }, [recentRuns]);

  const defaultSortDirectionFor = useCallback((key: SortKey): SortDirection => {
    switch (key) {
      case "lastRun":
        return "desc";
      default:
        return "asc";
    }
  }, []);

  const handleSort = useCallback((nextKey: SortKey) => {
    if (sortKey === nextKey) {
      setSortDirection((currentDirection) => (currentDirection === "asc" ? "desc" : "asc"));
      return;
    }

    setSortKey(nextKey);
    setSortDirection(defaultSortDirectionFor(nextKey));
  }, [defaultSortDirectionFor, sortKey]);

  const renderSortIcon = useCallback((key: SortKey) => {
    if (sortKey !== key) {
      return null;
    }

    return sortDirection === "asc"
      ? <ArrowUp className="h-3.5 w-3.5" />
      : <ArrowDown className="h-3.5 w-3.5" />;
  }, [sortDirection, sortKey]);

  const renderSortableHeader = useCallback((
    key: SortKey,
    label: string,
    className?: string,
  ) => (
    <TableHead
      className={className}
      aria-sort={
        sortKey === key
          ? sortDirection === "asc"
            ? "ascending"
            : "descending"
          : "none"
      }
    >
      <button
        type="button"
        className="inline-flex w-full items-center gap-1 text-left font-medium text-foreground transition-colors hover:text-foreground/80"
        onClick={() => handleSort(key)}
      >
        <span>{label}</span>
        {renderSortIcon(key)}
      </button>
    </TableHead>
  ), [handleSort, renderSortIcon, sortDirection, sortKey]);

  const jobRows = useMemo<JobTableRow[]>(() =>
    jobs.map((job) => {
      const rawActiveRun = activeRunsByJob[job.key];
      const activeLibraryScan =
        job.usesLibraryScanProgress && libraryFacetForJob(job.key)
          ? getActiveSession(libraryFacetForJob(job.key)!)
          : null;
      const recentRun = lastRunsByJob.get(job.key) ?? null;
      const activeRun = isStaleActiveRun(rawActiveRun, recentRun) ? null : (rawActiveRun ?? null);
      const lastRun = activeRun ?? recentRun;
      const status =
        activeRun?.status ??
        (activeLibraryScan ? jobStatusFromLibraryScanStatus(activeLibraryScan.status) : null) ??
        lastRun?.status ??
        "idle";
      const isTriggering = Boolean(triggeringKeys[job.key]);
      const hasActiveExecution = Boolean(activeRun) || Boolean(activeLibraryScan);
      const isDisabled = isRunButtonDisabled(hasActiveExecution, isTriggering);

      return {
        job,
        activeRun: activeRun ?? null,
        activeLibraryScan,
        lastRun,
        status,
        isDisabled,
      };
    }), [activeRunsByJob, getActiveSession, jobs, lastRunsByJob, triggeringKeys]);

  const sortedJobRows = useMemo(() => {
    const factor = sortDirection === "asc" ? 1 : -1;

    return [...jobRows].sort((left, right) => {
      const delta = (() => {
        switch (sortKey) {
          case "name":
            return compareText(left.job.displayName, right.job.displayName);
          case "nextRun":
            return compareMaybeDates(left.job.schedule.nextRunAt, right.job.schedule.nextRunAt);
          case "lastRun":
            return compareMaybeDates(
              left.lastRun?.completedAt ?? left.lastRun?.startedAt ?? null,
              right.lastRun?.completedAt ?? right.lastRun?.startedAt ?? null,
            );
          case "status":
            return (
              statusSortWeight(left.status) - statusSortWeight(right.status) ||
              compareText(runStatusLabel(left.status, t), runStatusLabel(right.status, t))
            );
          default:
            return 0;
        }
      })();

      if (delta !== 0) {
        return delta * factor;
      }

      return compareText(left.job.displayName, right.job.displayName);
    });
  }, [jobRows, sortDirection, sortKey, t]);

  const renderRows = (rows: JobTableRow[]) =>
    rows.map(({ job, lastRun, status, isDisabled }) => (
      <TableRow
        key={job.key}
        className="cursor-pointer hover:bg-muted/30"
        onClick={() => onSelectJob(job.key)}
      >
        <TableCell>
          <div className="space-y-1">
            <p className="font-medium text-foreground">{job.displayName}</p>
            <p className="text-xs text-muted-foreground">{job.description}</p>
          </div>
        </TableCell>
        <TableCell className="w-[14rem] max-w-[14rem] text-muted-foreground">
          {job.schedule.description}
        </TableCell>
        <TableCell className="w-[10.5rem] min-w-[10.5rem] text-muted-foreground">
          {renderTableDateTime(job.schedule.nextRunAt, t, dateTimeFormat)}
        </TableCell>
        <TableCell className="w-[10.5rem] min-w-[10.5rem] text-muted-foreground">
          {renderTableDateTime(
            lastRun?.completedAt ?? lastRun?.startedAt ?? null,
            t,
            dateTimeFormat,
          )}
        </TableCell>
        <TableCell>
          <span className={runStatusTone(status)}>{runStatusLabel(status, t)}</span>
        </TableCell>
        <TableCell>
          {job.manualTriggerAllowed ? (
            <Button
              size="sm"
              variant="default"
              disabled={isDisabled}
              onClick={(event) => {
                event.stopPropagation();
                onTriggerJob(job.key);
              }}
            >
              {t("jobs.action.run")}
            </Button>
          ) : null}
        </TableCell>
        </TableRow>
    ));

  const renderMobileCards = (rows: JobTableRow[]) =>
    rows.map(({ job, lastRun, status, isDisabled }) => (
      <div
        key={job.key}
        className="rounded-xl border border-border bg-card/55 p-4"
      >
        <div
          className="cursor-pointer space-y-3"
          onClick={() => onSelectJob(job.key)}
          role="button"
          tabIndex={0}
          onKeyDown={(event) => {
            if (event.key === "Enter" || event.key === " ") {
              event.preventDefault();
              onSelectJob(job.key);
            }
          }}
        >
          <div className="flex items-start justify-between gap-3">
            <div className="min-w-0 space-y-1">
              <p className="text-sm font-semibold text-foreground">{job.displayName}</p>
              <p className="text-xs leading-relaxed text-muted-foreground">{job.description}</p>
            </div>
            <span
              className={cn(
                "shrink-0 rounded-full border border-border px-2.5 py-1 text-[11px] font-medium",
                runStatusTone(status),
              )}
            >
              {runStatusLabel(status, t)}
            </span>
          </div>

          <div className="grid gap-3 sm:grid-cols-3">
            <div className="space-y-1">
              <p className="text-[11px] font-semibold uppercase tracking-[0.12em] text-muted-foreground">
                {t("jobs.column.schedule")}
              </p>
              <p className="text-sm text-foreground/85">{job.schedule.description}</p>
            </div>
            <div className="space-y-1">
              <p className="text-[11px] font-semibold uppercase tracking-[0.12em] text-muted-foreground">
                {t("jobs.column.nextRun")}
              </p>
              <p className="text-sm text-foreground/85">
                {formatDate(job.schedule.nextRunAt, t, dateTimeFormat)}
              </p>
            </div>
            <div className="space-y-1">
              <p className="text-[11px] font-semibold uppercase tracking-[0.12em] text-muted-foreground">
                {t("jobs.column.lastRun")}
              </p>
              <p className="text-sm text-foreground/85">
                {formatDate(
                  lastRun?.completedAt ?? lastRun?.startedAt ?? null,
                  t,
                  dateTimeFormat,
                )}
              </p>
            </div>
          </div>
        </div>

        <div className="mt-4 flex items-center justify-between gap-3 border-t border-border pt-3">
          <button
            type="button"
            className="text-xs font-medium text-muted-foreground underline-offset-4 hover:text-foreground hover:underline"
            onClick={() => onSelectJob(job.key)}
          >
            {t("jobs.recentRuns")}
          </button>
          {job.manualTriggerAllowed ? (
            <Button
              size="sm"
              variant="default"
              disabled={isDisabled}
              onClick={(event) => {
                event.stopPropagation();
                onTriggerJob(job.key);
              }}
            >
              {t("jobs.action.run")}
            </Button>
          ) : null}
        </div>
      </div>
    ));

  return (
    <>
      <div className="space-y-6">
        <Card>
          <CardHeader>
            <CardTitle>{t("jobs.title")}</CardTitle>
          </CardHeader>
          <CardContent>
            <div className="space-y-3 md:hidden">
              <div className="flex flex-wrap gap-2">
                <Button
                  type="button"
                  size="xs"
                  variant={sortKey === "name" ? "secondary" : "outline"}
                  onClick={() => handleSort("name")}
                >
                  {t("jobs.column.name")}
                  {renderSortIcon("name")}
                </Button>
                <Button
                  type="button"
                  size="xs"
                  variant={sortKey === "nextRun" ? "secondary" : "outline"}
                  onClick={() => handleSort("nextRun")}
                >
                  {t("jobs.column.nextRun")}
                  {renderSortIcon("nextRun")}
                </Button>
                <Button
                  type="button"
                  size="xs"
                  variant={sortKey === "lastRun" ? "secondary" : "outline"}
                  onClick={() => handleSort("lastRun")}
                >
                  {t("jobs.column.lastRun")}
                  {renderSortIcon("lastRun")}
                </Button>
                <Button
                  type="button"
                  size="xs"
                  variant={sortKey === "status" ? "secondary" : "outline"}
                  onClick={() => handleSort("status")}
                >
                  {t("jobs.column.status")}
                  {renderSortIcon("status")}
                </Button>
              </div>
              <div className="space-y-3">{renderMobileCards(sortedJobRows)}</div>
            </div>

            <div className="hidden md:block">
              <Table>
                <TableHeader>
                  <TableRow>
                    {renderSortableHeader("name", t("jobs.column.name"))}
                    <TableHead className="w-[14rem]">{t("jobs.column.schedule")}</TableHead>
                    {renderSortableHeader("nextRun", t("jobs.column.nextRun"), "w-[10.5rem]")}
                    {renderSortableHeader("lastRun", t("jobs.column.lastRun"), "w-[10.5rem]")}
                    {renderSortableHeader("status", t("jobs.column.status"))}
                    <TableHead />
                  </TableRow>
                </TableHeader>
                <TableBody>{renderRows(sortedJobRows)}</TableBody>
              </Table>
            </div>
          </CardContent>
        </Card>
      </div>

      <Sheet open={Boolean(selectedJob)} onOpenChange={(open) => onSelectJob(open ? selectedJobKey : null)}>
        <SheetContent side="right" className="sm:max-w-xl">
          {selectedJob ? (
            <>
              <SheetHeader>
                <SheetTitle>{selectedJob.displayName}</SheetTitle>
                <SheetDescription>{selectedJob.description}</SheetDescription>
              </SheetHeader>

              <div className="flex-1 space-y-4 overflow-y-auto px-4 pb-4">
                <div className="rounded-lg border border-border bg-muted/20 p-3">
                  <p className="text-xs uppercase tracking-wide text-muted-foreground">
                    {t("jobs.schedule")}
                  </p>
                  <p className="mt-1 text-sm text-foreground">{selectedJob.schedule.description}</p>
                  <p className="mt-1 text-xs text-muted-foreground">
                    {t("jobs.nextRunPrefix", {
                      value: formatDate(selectedJob.schedule.nextRunAt, t, dateTimeFormat),
                    })}
                  </p>
                </div>

                <div className="flex gap-2">
                  {(() => {
                    const activeRun = activeRunsByJob[selectedJob.key] ?? null;
                    const recentRun = lastRunsByJob.get(selectedJob.key) ?? null;
                    const activeLibraryScan =
                      selectedJob.usesLibraryScanProgress &&
                      libraryFacetForJob(selectedJob.key)
                        ? getActiveSession(libraryFacetForJob(selectedJob.key)!)
                        : null;
                    const effectiveActiveRun = isStaleActiveRun(activeRun, recentRun)
                      ? null
                      : activeRun;
                    const isTriggering = Boolean(triggeringKeys[selectedJob.key]);
                    const isDisabled = isRunButtonDisabled(
                      Boolean(effectiveActiveRun) || Boolean(activeLibraryScan),
                      isTriggering,
                    );

                    if (!selectedJob.manualTriggerAllowed) {
                      return null;
                    }

                    return (
                      <Button
                        variant="default"
                        onClick={() => onTriggerJob(selectedJob.key)}
                        disabled={isDisabled}
                      >
                        {t("jobs.action.runNow")}
                      </Button>
                    );
                  })()}
                </div>

                <div className="space-y-2">
                  <p className="text-sm font-medium text-foreground">{t("jobs.recentRuns")}</p>
                  {jobHistoryLoading ? (
                    <p className="text-sm text-muted-foreground">{t("jobs.loadingRecentRuns")}</p>
                  ) : selectedJobHistory.length === 0 ? (
                    <p className="text-sm text-muted-foreground">{t("jobs.noRunsYet")}</p>
                  ) : (
                    <div className="space-y-2">
                      {selectedJobHistory.map((run) => {
                        const healthCheckIssues = parseHealthCheckIssues(run);

                        return (
                          <div key={run.id} className="rounded-lg border border-border p-3">
                            <div className="flex items-start justify-between gap-3">
                              <div className="space-y-1">
                                <p className={runStatusTone(run.status)}>
                                  {runStatusLabel(run.status, t)}
                                </p>
                                <p className="text-xs text-muted-foreground">
                                  {t("jobs.startedAt", {
                                    value: formatDate(run.startedAt, t, dateTimeFormat),
                                  })}
                                </p>
                                <p className="text-xs text-muted-foreground">
                                  {t("jobs.completedAt", {
                                    value: formatDate(run.completedAt, t, dateTimeFormat),
                                  })}
                                </p>
                                {run.summaryText ? (
                                  <p className="text-sm text-foreground">{run.summaryText}</p>
                                ) : null}
                                {run.errorText ? (
                                  <p className="text-sm text-red-400">{run.errorText}</p>
                                ) : null}
                              </div>
                              <p className="text-xs text-muted-foreground">
                                {triggerSourceLabel(run.triggerSource, t)}
                              </p>
                            </div>

                            {healthCheckIssues.length > 0 ? (
                              <div className="mt-3 rounded-lg border border-border bg-muted/20 p-3">
                                <p className="text-xs uppercase tracking-wide text-muted-foreground">
                                  {t("jobs.healthCheckIssues")}
                                </p>
                                <div className="mt-2 space-y-2">
                                  {healthCheckIssues.map((issue, index) => (
                                    <div
                                      key={`${run.id}-${issue.source}-${index}`}
                                      className="rounded-md border border-border/80 bg-background/40 p-2"
                                    >
                                      <div className="flex items-start justify-between gap-3">
                                        <p className="text-sm font-medium text-foreground">
                                          {formatHealthCheckSource(issue.source)}
                                        </p>
                                        <span
                                          className={`text-xs ${healthCheckStatusTone(issue.status)}`}
                                        >
                                          {formatHealthCheckStatus(issue.status)}
                                        </span>
                                      </div>
                                      <p className="mt-1 text-sm text-muted-foreground">
                                        {issue.message}
                                      </p>
                                    </div>
                                  ))}
                                </div>
                              </div>
                            ) : null}
                          </div>
                        );
                      })}
                    </div>
                  )}
                </div>
              </div>
            </>
          ) : null}
        </SheetContent>
      </Sheet>
    </>
  );
}
