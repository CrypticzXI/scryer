import {
  ArrowDownToLine,
  CircleAlert,
  CircleOff,
  Link2,
  Loader2,
  Pause,
  Play,
  Trash2,
  XCircle,
} from "lucide-react";
import { Fragment, memo } from "react";

import { ActivityProgressBar } from "@/components/views/activity-progress-bar";
import {
  ActivityQueueDetailsPanel,
  ActivityQueueStatusBadge,
  ActivityQueueTitleContent,
} from "@/components/views/activity/queue-row-presentation";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { TableCell, TableRow } from "@/components/ui/table";
import type { DownloadQueueItem } from "@/lib/types";
import {
  type ActivityTab,
  formatBytes,
  getProgressBarColor,
  type QueueRowPresentation,
  type TranslateFn,
} from "@/lib/utils/activity-utils";
import { selectorId } from "@/lib/utils/dom-ids";

export type QueueTableRowProps = {
  queueItem: DownloadQueueItem;
  row: QueueRowPresentation;
  activeTab: ActivityTab;
  rowId: string;
  rowSelectorKey: string;
  detailId: string;
  isActionLoading: boolean;
  isRowBlocked: boolean;
  isRowFullyBusy: boolean;
  isManualImportPending: boolean;
  isExpanded: boolean;
  isImportSelected: boolean;
  rowActionVisualClass: string;
  t: TranslateFn;
  onToggleImportSelected: () => void;
  onToggleExpanded: () => void;
  onPause: () => void;
  onResume: () => void;
  onManualImport: () => void;
  onAssignTitle: () => void;
  onIgnore: () => void;
  onMarkFailedSearchAgain: () => void;
  onMarkFailedOnly: () => void;
  onRequestDelete: () => void;
};

export const QueueTableRow = memo(function QueueTableRow({
  queueItem,
  row,
  activeTab,
  rowId,
  rowSelectorKey,
  detailId,
  isActionLoading,
  isRowBlocked,
  isRowFullyBusy,
  isManualImportPending,
  isExpanded,
  isImportSelected,
  rowActionVisualClass,
  t,
  onToggleImportSelected,
  onToggleExpanded,
  onPause,
  onResume,
  onManualImport,
  onAssignTitle,
  onIgnore,
  onMarkFailedSearchAgain,
  onMarkFailedOnly,
  onRequestDelete,
}: QueueTableRowProps) {
  return (
    <Fragment>
      <TableRow
        id={selectorId("activity", activeTab, "row", rowSelectorKey)}
        data-ui="activity-row"
        data-activity-tab={activeTab}
        data-activity-row-id={rowId}
        data-activity-download-id={queueItem.id}
        data-activity-client-item-id={queueItem.downloadClientItemId}
        data-activity-title-id={queueItem.titleId ?? ""}
        data-activity-client-id={queueItem.clientId}
        data-activity-client-name={queueItem.clientName ?? ""}
        data-activity-client-type={queueItem.clientType}
      >
        {activeTab === "import" ? (
          <TableCell className="w-12 min-w-12 align-middle">
            <Checkbox
              checked={isImportSelected}
              aria-label={t("activity.selectImportItem")}
              onCheckedChange={onToggleImportSelected}
            />
          </TableCell>
        ) : null}
        <TableCell className="w-[28%] min-w-72">
          <ActivityQueueTitleContent
            displayTitle={row.displayTitle}
            releaseTitle={row.releaseTitle}
          />
        </TableCell>
        <TableCell className="w-36 min-w-36 align-middle">
          <p className="break-words whitespace-normal text-sm">
            {queueItem.clientName || queueItem.clientType}
          </p>
          <p className="text-xs text-muted-foreground">{queueItem.clientType}</p>
        </TableCell>
        <TableCell className="w-44 min-w-44 align-middle">
          <ActivityQueueStatusBadge
            stateKey={row.displayStateKey}
            statusLabel={row.statusLabel}
            isExpandable={row.hasExpandableDetails}
            isExpanded={isExpanded}
            detailId={detailId}
            expandLabel={t(
              isExpanded ? "queue.hideDetails" : "queue.showDetails",
            )}
            onToggle={onToggleExpanded}
          />
          {(queueItem.deleteErrorMessage || queueItem.importErrorMessage) &&
            !row.hasStatusDetails && (
            <p
              className="mt-1 max-w-full break-words whitespace-normal text-xs text-rose-400"
              title={queueItem.deleteErrorMessage ?? queueItem.importErrorMessage ?? ""}
            >
              {queueItem.deleteErrorMessage ?? queueItem.importErrorMessage}
            </p>
          )}
        </TableCell>
        {activeTab === "activity" || activeTab === "import" ? (
          <TableCell className="w-48 min-w-48 align-middle">
            <ActivityProgressBar
              percent={row.percent}
              remainingLabel={row.remainingLabel}
              colorClass={getProgressBarColor(row.displayStateKey)}
            />
          </TableCell>
        ) : null}
        <TableCell className="w-24 min-w-24 align-middle">
          {formatBytes(queueItem.sizeBytes)}
        </TableCell>
        <TableCell className="w-44 min-w-44 align-middle text-right">
          <div className="flex items-center justify-end gap-2">
            {row.canPause && (
              <Button
                type="button"
                size="sm"
                variant="secondary"
                className={`h-10 w-10 border border-border/50 bg-muted/70 text-foreground hover:bg-accent/90 ${rowActionVisualClass}`}
                disabled={isRowFullyBusy}
                title={t("queue.pause")}
                aria-label={t("queue.pause")}
                onClick={() => {
                  if (
                    isActionLoading || isRowBlocked
                  ) {
                    return;
                  }
                  onPause();
                }}
              >
                <Pause className="h-6 w-6" />
              </Button>
            )}
            {row.canResume && (
              <Button
                type="button"
                size="sm"
                variant="secondary"
                className={`h-10 w-10 border border-border/50 bg-muted/70 text-foreground hover:bg-accent/90 ${rowActionVisualClass}`}
                disabled={isRowFullyBusy}
                title={t("queue.resume")}
                aria-label={t("queue.resume")}
                onClick={() => {
                  if (
                    isActionLoading || isRowBlocked
                  ) {
                    return;
                  }
                  onResume();
                }}
              >
                <Play className="h-6 w-6" />
              </Button>
            )}
            {(row.canInteractiveManualImport || row.canDirectManualImport) && (
              <Button
                id={selectorId("activity", activeTab, "manual-import", rowSelectorKey)}
                type="button"
                size="sm"
                variant="secondary"
                className={`h-10 w-10 border border-emerald-500/60 dark:border-emerald-500/50 bg-emerald-600/20 dark:bg-emerald-600/15 text-emerald-700 dark:text-emerald-200 hover:bg-emerald-600/30 dark:hover:bg-emerald-600/25 ${rowActionVisualClass}`}
                disabled={isRowFullyBusy}
                title={
                  isManualImportPending
                    ? t("queue.manualImporting")
                    : t("queue.manualImportTooltip")
                }
                aria-label={
                  isManualImportPending
                    ? t("queue.manualImporting")
                    : t("queue.manualImportTooltip")
                }
                onClick={() => {
                  if (
                    isActionLoading || isRowBlocked
                  ) {
                    return;
                  }
                  onManualImport();
                }}
              >
                {isManualImportPending ? (
                  <Loader2 className="h-5 w-5 animate-spin" />
                ) : (
                  <ArrowDownToLine className="h-5 w-5" />
                )}
              </Button>
            )}
            {row.canAssignTitle && (
              <Button
                id={selectorId("activity", activeTab, "assign-title", rowSelectorKey)}
                type="button"
                size="sm"
                variant="secondary"
                className={`h-10 w-10 border border-amber-500/60 bg-amber-600/15 text-amber-200 hover:bg-amber-600/25 ${rowActionVisualClass}`}
                disabled={isRowFullyBusy}
                title={
                  row.trackedMatchTypeKey === "unmatched" || !queueItem.titleId
                    ? t("queue.assignTitle")
                    : t("queue.reassignTitle")
                }
                aria-label={
                  row.trackedMatchTypeKey === "unmatched" || !queueItem.titleId
                    ? t("queue.assignTitle")
                    : t("queue.reassignTitle")
                }
                onClick={() => {
                  if (
                    isActionLoading || isRowBlocked
                  ) {
                    return;
                  }
                  onAssignTitle();
                }}
              >
                <Link2 className="h-5 w-5" />
              </Button>
            )}
            {row.canIgnore && (
              <Button
                type="button"
                size="sm"
                variant="secondary"
                className={`h-10 w-10 border border-border/50 bg-muted/70 text-foreground hover:bg-accent/90 ${rowActionVisualClass}`}
                disabled={isRowFullyBusy}
                title={t("queue.ignore")}
                aria-label={t("queue.ignore")}
                onClick={() => {
                  if (
                    isActionLoading || isRowBlocked
                  ) {
                    return;
                  }
                  onIgnore();
                }}
              >
                <CircleOff className="h-5 w-5" />
              </Button>
            )}
            {row.canMarkFailed && (
              <Button
                type="button"
                size="sm"
                variant="secondary"
                className={`h-10 w-10 border border-orange-500/50 bg-orange-600/15 text-orange-200 hover:bg-orange-600/25 ${rowActionVisualClass}`}
                disabled={isRowFullyBusy}
                title={t("queue.markFailedSearchAgain")}
                aria-label={t("queue.markFailedSearchAgain")}
                onClick={() => {
                  if (isActionLoading || isRowBlocked) {
                    return;
                  }
                  onMarkFailedSearchAgain();
                }}
              >
                <CircleAlert className="h-5 w-5" />
              </Button>
            )}
            {row.canMarkFailed && (
              <Button
                type="button"
                size="sm"
                variant="secondary"
                className={`h-10 w-10 border border-rose-500/50 bg-rose-600/15 text-rose-200 hover:bg-rose-600/25 ${rowActionVisualClass}`}
                disabled={isRowFullyBusy}
                title={t("queue.markFailedOnly")}
                aria-label={t("queue.markFailedOnly")}
                onClick={() => {
                  if (isActionLoading || isRowBlocked) {
                    return;
                  }
                  onMarkFailedOnly();
                }}
              >
                <XCircle className="h-5 w-5" />
              </Button>
            )}
            <Button
              type="button"
              size="sm"
              variant="secondary"
              className={`h-10 w-10 border border-rose-500/50 bg-rose-600/15 text-rose-300 hover:bg-rose-600/25 ${rowActionVisualClass}`}
              disabled={isRowFullyBusy}
              title={t("label.delete")}
              aria-label={t("label.delete")}
              onClick={() => {
                if (
                  isActionLoading || isRowBlocked
                ) {
                  return;
                }
                onRequestDelete();
              }}
            >
              <Trash2 className="h-6 w-6" />
            </Button>
          </div>
        </TableCell>
      </TableRow>
      {row.hasExpandableDetails && isExpanded ? (
        <TableRow>
          <TableCell
            colSpan={activeTab === "activity" ? 6 : activeTab === "import" ? 7 : 5}
            className="bg-muted/10 p-3"
          >
            <ActivityQueueDetailsPanel
              detailId={detailId}
              releaseTitle={row.releaseTitle}
              errorCode={queueItem.importErrorCode}
              failureReason={row.failureReason}
              t={t}
            />
          </TableCell>
        </TableRow>
      ) : null}
    </Fragment>
  );
});
