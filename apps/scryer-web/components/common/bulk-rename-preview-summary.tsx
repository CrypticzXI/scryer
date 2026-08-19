import { ArrowDown } from "lucide-react";
import { useTranslate } from "@/lib/context/translate-context";
import type { MediaRenamePlan } from "@/components/common/media-rename-plan-panel";
import {
  BULK_RENAME_ITEM_SAMPLE_LIMIT,
  type BulkRenameSummary,
} from "@/lib/hooks/use-bulk-rename";
import type { TitleRecord } from "@/lib/types";

type BulkRenamePreviewSummaryProps = {
  titles: TitleRecord[];
  plansByTitleId: Record<string, MediaRenamePlan>;
  summary: BulkRenameSummary | null;
  loading: boolean;
  error: string | null;
};

export function BulkRenamePreviewSummary({
  titles,
  plansByTitleId,
  summary,
  loading,
  error,
}: BulkRenamePreviewSummaryProps) {
  const t = useTranslate();

  if (loading) {
    return (
      <p className="rounded border border-border bg-muted/40 px-3 py-2 text-xs text-muted-foreground">
        {t("rename.previewing")}
      </p>
    );
  }

  // Previews are already scoped to the sample limit, so this only guards
  // against a server that returned more than was asked for.
  let remainingSampleSlots = BULK_RENAME_ITEM_SAMPLE_LIMIT;
  const entries: { title: TitleRecord; items: MediaRenamePlan["items"] }[] = [];
  for (const title of titles) {
    const plan = plansByTitleId[title.id];
    if (!plan || remainingSampleSlots === 0) {
      continue;
    }
    const items = plan.items.slice(0, remainingSampleSlots);
    if (items.length === 0) {
      continue;
    }
    remainingSampleSlots -= items.length;
    entries.push({ title, items });
  }
  const sampledCount = BULK_RENAME_ITEM_SAMPLE_LIMIT - remainingSampleSlots;

  return (
    <div className="space-y-3">
      {error ? (
        <div className="rounded border border-destructive/40 bg-destructive/10 px-3 py-2">
          <p className="text-xs text-destructive/90">{error}</p>
        </div>
      ) : null}
      {summary ? (
        <>
          <div
            data-ui="bulk-rename-plan-summary"
            className="text-xs text-muted-foreground"
          >
            {t("rename.planSummary", {
              total: summary.total,
              renamable: summary.renamable,
              noop: summary.noop,
              conflicts: summary.conflicts,
              errors: summary.errors,
            })}
          </div>
          {summary.renamable === 0 ? (
            <p className="rounded border border-border bg-muted/40 px-3 py-2 text-xs text-muted-foreground">
              {t("rename.noRenamableFiles")}
            </p>
          ) : (
            <div className="max-h-96 space-y-3 overflow-auto rounded-lg border border-border p-2">
              {entries.map(({ title, items }) => {
                return (
                  <div key={title.id} className="space-y-1">
                    <div className="text-xs font-semibold text-card-foreground">
                      {title.name}
                    </div>
                    <ul className="text-xs">
                      {items.map((item, index) => (
                        <li
                          key={`${item.collectionId ?? "none"}-${item.currentPath ?? ""}-${index}`}
                          className="border-t border-border/60 px-2 py-2 first:border-t-0"
                        >
                          {/* Shrink-wrapping the pair keeps the arrow centered
                              on the paths themselves, not the whole row. */}
                          <div className="flex w-fit max-w-full flex-col items-start">
                            <div
                              data-ui="bulk-rename-plan-current-path"
                              className="break-all font-[var(--font-code)] text-muted-foreground"
                            >
                              {item.currentPath || "—"}
                            </div>
                            <ArrowDown
                              aria-hidden
                              className="my-1 h-3.5 w-3.5 self-center text-[var(--scry-accent-text)]"
                            />
                            <div
                              data-ui="bulk-rename-plan-proposed-path"
                              className="break-all font-[var(--font-code)] text-card-foreground"
                            >
                              {item.proposedPath ?? "—"}
                            </div>
                          </div>
                        </li>
                      ))}
                    </ul>
                  </div>
                );
              })}
            </div>
          )}
          {summary.renamable > sampledCount ? (
            <p
              data-ui="bulk-rename-plan-sample-note"
              className="text-xs text-muted-foreground"
            >
              {t("rename.sampleNote", {
                shown: sampledCount,
                renamable: summary.renamable,
              })}
            </p>
          ) : null}
        </>
      ) : null}
    </div>
  );
}
