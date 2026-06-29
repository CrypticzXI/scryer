import { useState, type CSSProperties } from "react";
import {
  ArrowLeftRight,
  CircleCheckBig,
  Download,
  Eye,
  Library,
  Loader,
  RotateCcw,
  Route,
  Rss,
  Server,
  TriangleAlert,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Progress } from "@/components/ui/progress";
import type { UseExternalImportSetupReturn } from "@/lib/hooks/use-external-import-setup";

interface SetupImportSummaryViewProps {
  wizard: UseExternalImportSetupReturn;
  t: (key: string, values?: Record<string, unknown>) => string;
}

interface StatRow {
  key: string;
  icon: LucideIcon;
  title: string;
  detail: string;
  count: number;
}

const accentTileStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
  flex: "none",
  width: 38,
  height: 38,
  borderRadius: 10,
  background: "rgba(var(--scry-accent-rgb), 0.12)",
  border: "1px solid var(--scry-baccent)",
  color: "var(--scry-accent-text)",
};

const cardStyle: CSSProperties = {
  border: "1px solid var(--scry-border)",
  borderRadius: 16,
  background: "rgba(10, 17, 32, 0.5)",
  padding: "14px 22px",
};

export default function SetupImportSummaryView({
  wizard,
  t,
}: SetupImportSummaryViewProps) {
  const {
    summary,
    aggregateProgress,
    warmupComplete,
    warmupFailed,
    warmupErrorMessage,
    retryWarmup,
    mappingReady,
    preview,
    previewError,
    loadPreview,
  } = wizard;

  // The Summary also needs the preview loaded (it yields the root mappings for
  // finalize). If that load fails — distinct endpoint from the warmup, so it can
  // fail even when the warmup itself completed — treat it as a recoverable
  // failure rather than silently leaving Finish disabled with no explanation.
  const previewBlocked = !preview && Boolean(previewError);
  const showFailure = warmupFailed || previewBlocked;
  // Map raw warmup/preview errors to actionable copy — raw GraphQL/transport
  // strings (e.g. "[GraphQL] not found: no warmup session …", which means the
  // session was pruned after a restart/idle) must never reach the UI.
  const rawFailure = warmupErrorMessage ?? previewError;
  const sessionExpired =
    !!rawFailure && /no warmup session/i.test(rawFailure);
  const failureDetail = sessionExpired
    ? t("setup.importWarmupSessionExpired")
    : rawFailure?.replace(/^\[(?:GraphQL|Network)\]\s*/i, "").trim() ||
      t("setup.importWarmupFailedDetail");

  const [retrying, setRetrying] = useState(false);
  const handleRetry = async () => {
    setRetrying(true);
    try {
      // Warmup failure → re-warm; preview-only failure → just refetch preview.
      if (warmupFailed) await retryWarmup();
      else await loadPreview();
    } finally {
      setRetrying(false);
    }
  };

  const rows: StatRow[] = [
    {
      key: "libraries",
      icon: Library,
      title: t("setup.summaryLibraries"),
      detail: t("setup.summaryLibrariesDetail"),
      count: summary.libraryCount,
    },
    {
      key: "instances",
      icon: Server,
      title: t("setup.summaryInstancesConnected"),
      detail: t("setup.summaryInstancesConnectedDetail", {
        sonarr: summary.sonarrCount,
        radarr: summary.radarrCount,
      }),
      count: summary.instancesConnected,
    },
    {
      key: "roots",
      icon: Route,
      title: t("setup.summaryRootsMapped"),
      detail: t("setup.summaryRootsMappedDetail"),
      count: summary.rootsMapped,
    },
    ...(summary.pathsRemapped > 0
      ? [
          {
            key: "remapped",
            icon: ArrowLeftRight,
            title: t("setup.summaryPathsRemapped"),
            detail: t("setup.summaryPathsRemappedDetail"),
            count: summary.pathsRemapped,
          },
        ]
      : []),
    {
      key: "clients",
      icon: Download,
      title: t("setup.downloadClients"),
      detail: t("setup.summaryClientsMerged"),
      count: summary.downloadClients,
    },
    {
      key: "indexers",
      icon: Rss,
      title: t("setup.indexers"),
      detail: t("setup.summaryIndexersEnabled"),
      count: summary.indexers,
    },
  ];

  const titlesFetched = aggregateProgress?.titlesFetched ?? 0;
  const titlesTotal = aggregateProgress?.titlesTotal ?? 0;
  const pct =
    titlesTotal > 0
      ? Math.round((titlesFetched / titlesTotal) * 100)
      : warmupComplete
        ? 100
        : 0;

  return (
    <div data-slot="setup-import-summary" className="flex flex-col gap-5">
      {/* ── Stat list card ─────────────────────────────────────────────── */}
      <div style={cardStyle}>
        {rows.map((row, index) => {
          const Icon = row.icon;
          return (
            <div
              key={row.key}
              style={{
                display: "flex",
                alignItems: "center",
                gap: 14,
                padding: "15px 0",
                borderTop:
                  index === 0 ? undefined : "1px solid var(--scry-hover)",
              }}
            >
              <div style={accentTileStyle}>
                <Icon size={18} strokeWidth={2} aria-hidden />
              </div>
              <div style={{ flex: 1, minWidth: 0 }}>
                <div
                  style={{
                    fontSize: 14.5,
                    fontWeight: 600,
                    color: "#f1f5ff",
                  }}
                >
                  {row.title}
                </div>
                <div
                  style={{
                    fontSize: 12.5,
                    color: "var(--scry-muted3)",
                  }}
                >
                  {row.detail}
                </div>
              </div>
              <div
                style={{
                  flex: "none",
                  fontFamily: "var(--font-space-grotesk)",
                  fontWeight: 700,
                  fontSize: 22,
                  color: "#fff",
                  letterSpacing: "-0.02em",
                }}
              >
                {row.count}
              </div>
            </div>
          );
        })}
      </div>

      {/* ── Monitored-status fetch card ────────────────────────────────── */}
      <div style={cardStyle}>
        {showFailure && !retrying ? (
          <div className="flex flex-col gap-3">
            <div style={{ display: "flex", alignItems: "center", gap: 14 }}>
              <div
                style={{
                  ...accentTileStyle,
                  background: "rgba(248, 113, 113, 0.12)",
                  border: "1px solid rgba(248, 113, 113, 0.4)",
                  color: "#f87171",
                }}
              >
                <TriangleAlert size={18} strokeWidth={2} aria-hidden />
              </div>
              <div style={{ flex: 1, minWidth: 0 }}>
                <div
                  style={{ fontSize: 14.5, fontWeight: 600, color: "#f1f5ff" }}
                >
                  {sessionExpired
                    ? t("setup.importWarmupSessionExpiredTitle")
                    : t("setup.importWarmupFailedTitle")}
                </div>
                <div style={{ fontSize: 12.5, color: "var(--scry-muted3)" }}>
                  {failureDetail}
                </div>
              </div>
              <Button
                type="button"
                variant="outline"
                size="sm"
                disabled={retrying}
                onClick={() => void handleRetry()}
              >
                {retrying ? (
                  <Loader className="h-4 w-4 animate-spin" />
                ) : (
                  <RotateCcw className="h-4 w-4" />
                )}
                {retrying
                  ? t("setup.importWarmupRetrying")
                  : t("setup.retry")}
              </Button>
            </div>
          </div>
        ) : (
          <>
        <div style={{ display: "flex", alignItems: "center", gap: 14 }}>
          <div style={accentTileStyle}>
            <Eye size={18} strokeWidth={2} aria-hidden />
          </div>
          <div style={{ flex: 1, minWidth: 0 }}>
            <div
              style={{
                fontSize: 14.5,
                fontWeight: 600,
                color: "#f1f5ff",
              }}
            >
              {warmupComplete
                ? t("setup.monitoredStatusSynced")
                : t("setup.fetchingMonitoredStatus")}
            </div>
            <div
              style={{
                fontSize: 12.5,
                color: "var(--scry-muted3)",
              }}
            >
              {warmupComplete
                ? t("setup.monitoredStatusSyncedDetail")
                : t("setup.fetchingMonitoredStatusDetail")}
            </div>
          </div>
          <div
            style={{
              flex: "none",
              fontFamily: "var(--font-space-grotesk)",
              fontWeight: 700,
              fontSize: 22,
              color: warmupComplete ? "#4ade80" : "#fff",
              letterSpacing: "-0.02em",
            }}
          >
            {pct}%
          </div>
        </div>

        <Progress
          value={pct}
          className="mt-3.5 h-2"
          style={{ background: "var(--scry-page2)" }}
          indicatorClassName={warmupComplete ? "bg-emerald-400" : undefined}
        />

        <div
          style={{
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
            gap: 12,
            marginTop: 12,
          }}
        >
          <div
            style={{
              display: "inline-flex",
              alignItems: "center",
              gap: 7,
              fontSize: 12.5,
              color: warmupComplete ? "#4ade80" : "var(--scry-accent-text)",
            }}
          >
            {warmupComplete ? (
              <CircleCheckBig size={15} strokeWidth={2} aria-hidden />
            ) : (
              <Loader
                size={15}
                strokeWidth={2}
                aria-hidden
                className="animate-spin"
              />
            )}
            <span>
              {warmupComplete
                ? t("setup.importGateDoneHint")
                : t("setup.importGateHint")}
            </span>
          </div>
          <div
            style={{
              flex: "none",
              fontFamily: "var(--font-code)",
              fontSize: 12.5,
              color: "var(--scry-faint)",
              whiteSpace: "nowrap",
            }}
          >
            {t("setup.titlesFetched", {
              fetched: titlesFetched.toLocaleString(),
              total: titlesTotal.toLocaleString(),
            })}
          </div>
        </div>
          </>
        )}
      </div>

      {/* Unmapped-root notice: finalize requires every detected source root to
          be mapped, so explain a disabled Finish instead of failing later. */}
      {warmupComplete && !mappingReady ? (
        <div
          style={{
            ...cardStyle,
            borderColor: "rgba(251, 191, 36, 0.4)",
            display: "flex",
            alignItems: "center",
            gap: 10,
            fontSize: 12.5,
            color: "#fbbf24",
          }}
        >
          <TriangleAlert
            size={16}
            strokeWidth={2}
            aria-hidden
            style={{ flex: "none" }}
          />
          <span>{t("setup.importUnmappedRootsNotice")}</span>
        </div>
      ) : null}
    </div>
  );
}
