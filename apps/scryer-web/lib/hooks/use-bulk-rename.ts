import * as React from "react";
import type { Client } from "urql";
import { applyMediaRenameMutation } from "@/lib/graphql/mutations";
import { mediaRenamePreviewQuery } from "@/lib/graphql/queries";
import type { MediaRenamePlan } from "@/components/common/media-rename-plan-panel";
import type { TitleRecord } from "@/lib/types";
import type { Translate } from "@/components/root/types";
import type { SetGlobalStatus } from "@/lib/context/global-status-context";

const BULK_RENAME_PREVIEW_CONCURRENCY = 4;
export const BULK_RENAME_ITEM_SAMPLE_LIMIT = 50;

type UseBulkRenameArgs = {
  selectedTitles: TitleRecord[];
  bulkActionBusy: boolean;
  setBulkActionBusy: React.Dispatch<React.SetStateAction<boolean>>;
  client: Client;
  t: Translate;
  setGlobalStatus: SetGlobalStatus;
  recordCriticalCatalogMutation: () => void;
  reloadTitles: () => Promise<TitleRecord[] | null>;
  setSelectedTitleIds: React.Dispatch<React.SetStateAction<Set<string>>>;
  batchFailureDetail: (error: unknown) => string | null;
  withFailureDetail: (message: string, detail: string | null) => string;
};

export type BulkRenameSummary = {
  total: number;
  renamable: number;
  noop: number;
  conflicts: number;
  errors: number;
};

export function useBulkRename({
  selectedTitles,
  bulkActionBusy,
  setBulkActionBusy,
  client,
  t,
  setGlobalStatus,
  recordCriticalCatalogMutation,
  reloadTitles,
  setSelectedTitleIds,
  batchFailureDetail,
  withFailureDetail,
}: UseBulkRenameArgs) {
  const [bulkRenameDialogOpen, setBulkRenameDialogOpen] = React.useState(false);
  const [bulkRenamePreviewLoading, setBulkRenamePreviewLoading] =
    React.useState(false);
  const [bulkRenamePreviewError, setBulkRenamePreviewError] = React.useState<
    string | null
  >(null);
  const [bulkRenamePlansByTitleId, setBulkRenamePlansByTitleId] =
    React.useState<Record<string, MediaRenamePlan>>({});

  const closeBulkRenameDialog = React.useCallback(() => {
    setBulkRenameDialogOpen(false);
    setBulkRenamePreviewLoading(false);
    setBulkRenamePreviewError(null);
    setBulkRenamePlansByTitleId({});
  }, []);

  React.useEffect(() => {
    if (!bulkRenameDialogOpen) {
      return;
    }

    const targets = [...selectedTitles];
    if (targets.length === 0) {
      setBulkRenamePreviewLoading(false);
      setBulkRenamePreviewError(null);
      setBulkRenamePlansByTitleId({});
      return;
    }

    let cancelled = false;
    setBulkRenamePreviewLoading(true);
    setBulkRenamePreviewError(null);
    setBulkRenamePlansByTitleId({});

    const loadPreviews = async () => {
      const nextPlansByTitleId: Record<string, MediaRenamePlan> = {};
      const failedTitles: string[] = [];
      let firstFailureDetail: string | null = null;
      const queue = [...targets];
      // The dialog only ever shows a sample, so each request asks for what is
      // still missing from it. Plan counts and the fingerprint describe every
      // file regardless of how few items come back.
      let sampledItems = 0;

      const worker = async () => {
        for (;;) {
          const title = queue.shift();
          if (!title || cancelled) {
            return;
          }
          try {
            const result = await client
              .query<{ mediaRenamePreview: MediaRenamePlan }>(
                mediaRenamePreviewQuery,
                {
                  input: {
                    facet: title.facet,
                    titleId: title.id,
                    dryRun: true,
                    renamableOnly: true,
                    maxItems: Math.max(
                      0,
                      BULK_RENAME_ITEM_SAMPLE_LIMIT - sampledItems,
                    ),
                  },
                },
                { requestPolicy: "network-only" },
              )
              .toPromise();
            if (result.error || !result.data?.mediaRenamePreview) {
              throw result.error ?? new Error("rename preview failed");
            }
            const plan = result.data.mediaRenamePreview;
            sampledItems += plan.items.length;
            nextPlansByTitleId[title.id] = plan;
          } catch (error) {
            failedTitles.push(title.name || title.id);
            firstFailureDetail ??= batchFailureDetail(error);
          }
        }
      };

      await Promise.all(
        Array.from(
          { length: Math.min(BULK_RENAME_PREVIEW_CONCURRENCY, targets.length) },
          worker,
        ),
      );
      if (cancelled) {
        return;
      }

      setBulkRenamePlansByTitleId(nextPlansByTitleId);
      if (failedTitles.length > 0) {
        setBulkRenamePreviewError(
          withFailureDetail(
            t("status.bulkRenamePreviewFailed", {
              failed: failedTitles.length,
            }),
            failedTitles.slice(0, 5).join(", ") || firstFailureDetail,
          ),
        );
      } else {
        setBulkRenamePreviewError(null);
      }
      setBulkRenamePreviewLoading(false);
    };

    void loadPreviews();
    return () => {
      cancelled = true;
    };
  }, [
    batchFailureDetail,
    bulkRenameDialogOpen,
    client,
    selectedTitles,
    t,
    withFailureDetail,
  ]);

  const bulkRenameSummary = React.useMemo<BulkRenameSummary | null>(() => {
    const plans = selectedTitles
      .map((title) => bulkRenamePlansByTitleId[title.id])
      .filter(Boolean);
    if (plans.length === 0) {
      return null;
    }
    return plans.reduce<BulkRenameSummary>(
      (summary, plan) => ({
        total: summary.total + plan.total,
        renamable: summary.renamable + plan.renamable,
        noop: summary.noop + plan.noop,
        conflicts: summary.conflicts + plan.conflicts,
        errors: summary.errors + plan.errors,
      }),
      { total: 0, renamable: 0, noop: 0, conflicts: 0, errors: 0 },
    );
  }, [bulkRenamePlansByTitleId, selectedTitles]);

  const bulkRenameConfirmDisabled =
    bulkActionBusy ||
    selectedTitles.length === 0 ||
    bulkRenamePreviewLoading ||
    !bulkRenameSummary ||
    bulkRenameSummary.renamable === 0;

  const confirmBulkRenameTitles = React.useCallback(async () => {
    const targets = selectedTitles.filter((title) => {
      const plan = bulkRenamePlansByTitleId[title.id];
      return plan !== undefined && plan.renamable > 0;
    });
    if (targets.length === 0 || bulkActionBusy) {
      return;
    }

    setBulkActionBusy(true);
    try {
      recordCriticalCatalogMutation();
      let appliedFiles = 0;
      const succeededIds: string[] = [];
      const failedIds: string[] = [];
      let firstFailureDetail: string | null = null;

      for (const title of targets) {
        const plan = bulkRenamePlansByTitleId[title.id];
        try {
          const result = await client
            .mutation<{
              applyMediaRename: {
                applied: number;
                skipped: number;
                failed: number;
              };
            }>(applyMediaRenameMutation, {
              input: {
                facet: title.facet,
                titleId: title.id,
                fingerprint: plan.fingerprint,
              },
            })
            .toPromise();
          if (result.error) {
            throw result.error;
          }
          const payload = result.data?.applyMediaRename;
          appliedFiles += payload?.applied ?? 0;
          if ((payload?.failed ?? 0) > 0) {
            failedIds.push(title.id);
          } else {
            succeededIds.push(title.id);
          }
        } catch (error) {
          failedIds.push(title.id);
          firstFailureDetail ??= batchFailureDetail(error);
        }
      }

      await reloadTitles();
      setSelectedTitleIds(new Set(failedIds));
      closeBulkRenameDialog();

      if (succeededIds.length === 0) {
        setGlobalStatus(
          withFailureDetail(t("status.bulkRenameFailed"), firstFailureDetail),
        );
        return;
      }
      if (failedIds.length > 0) {
        setGlobalStatus(
          withFailureDetail(
            t("status.bulkRenamePartial", {
              count: succeededIds.length,
              failed: failedIds.length,
            }),
            firstFailureDetail,
          ),
        );
        return;
      }
      setGlobalStatus(
        t("status.bulkRenameSuccess", {
          files: appliedFiles,
          count: succeededIds.length,
        }),
      );
    } catch (error) {
      setGlobalStatus(
        withFailureDetail(
          t("status.bulkRenameFailed"),
          batchFailureDetail(error),
        ),
      );
    } finally {
      setBulkActionBusy(false);
    }
  }, [
    batchFailureDetail,
    bulkActionBusy,
    bulkRenamePlansByTitleId,
    client,
    closeBulkRenameDialog,
    recordCriticalCatalogMutation,
    reloadTitles,
    selectedTitles,
    setBulkActionBusy,
    setGlobalStatus,
    setSelectedTitleIds,
    withFailureDetail,
    t,
  ]);

  const openBulkTitleRename = React.useCallback(() => {
    if (selectedTitles.length === 0 || bulkActionBusy) {
      return;
    }
    setBulkRenamePreviewLoading(false);
    setBulkRenamePreviewError(null);
    setBulkRenamePlansByTitleId({});
    setBulkRenameDialogOpen(true);
  }, [bulkActionBusy, selectedTitles.length]);

  return {
    bulkRenameDialogOpen,
    setBulkRenameDialogOpen,
    bulkRenamePreviewLoading,
    setBulkRenamePreviewLoading,
    bulkRenamePreviewError,
    setBulkRenamePreviewError,
    bulkRenamePlansByTitleId,
    setBulkRenamePlansByTitleId,
    bulkRenameSummary,
    bulkRenameConfirmDisabled,
    closeBulkRenameDialog,
    confirmBulkRenameTitles,
    openBulkTitleRename,
  };
}
