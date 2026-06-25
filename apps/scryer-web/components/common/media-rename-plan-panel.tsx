import { Button } from "@/components/ui/button";
import { useTranslate } from "@/lib/context/translate-context";

export type MediaRenamePlanItem = {
  collectionId?: string | null;
  seriesMovieLinkIds?: string[];
  currentPath?: string | null;
  proposedPath?: string | null;
  normalizedFilename?: string | null;
  collision?: boolean;
  reasonCode?: string | null;
  writeAction?: string | null;
  sourceSizeBytes?: number | null;
  sourceMtimeUnixMs?: string | null;
};

export type MediaRenamePlan = {
  facet?: string;
  titleId?: string | null;
  template?: string;
  collisionPolicy?: string;
  missingMetadataPolicy?: string;
  fingerprint: string;
  total: number;
  renamable: number;
  noop: number;
  conflicts: number;
  errors: number;
  items: MediaRenamePlanItem[];
};

export function MediaRenamePlanPanel({
  plan,
  applying,
  applyDisabled,
  onApply,
  applyButtonId,
}: {
  plan: MediaRenamePlan;
  applying: boolean;
  applyDisabled: boolean;
  onApply: () => void;
  applyButtonId?: string;
}) {
  const t = useTranslate();

  return (
    <div className="mt-3 space-y-3">
      <div className="text-sm text-muted-foreground">
        {t("rename.planSummary", {
          total: plan.total,
          renamable: plan.renamable,
          noop: plan.noop,
          conflicts: plan.conflicts,
          errors: plan.errors,
        })}
      </div>
      <div className="max-h-72 overflow-auto rounded-lg border border-border">
        <table className="min-w-full text-sm">
          <thead className="bg-card/70 text-muted-foreground">
            <tr>
              <th className="px-3 py-2 text-left font-medium">{t("rename.currentPath")}</th>
              <th className="px-3 py-2 text-left font-medium">{t("rename.proposedPath")}</th>
            </tr>
          </thead>
          <tbody>
            {plan.items.map((item, index) => (
              <tr
                key={`${item.collectionId ?? "none"}-${item.currentPath ?? ""}-${index}`}
                className="border-t border-border"
              >
                <td className="px-3 py-2 align-top font-mono text-xs text-muted-foreground">
                  {item.currentPath || "—"}
                </td>
                <td className="px-3 py-2 align-top font-mono text-xs text-muted-foreground">
                  {item.proposedPath ?? "—"}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <div className="flex justify-end">
        <Button id={applyButtonId} size="sm" type="button" onClick={onApply} disabled={applyDisabled}>
          {applying ? t("rename.applying") : t("rename.applyButton")}
        </Button>
      </div>
    </div>
  );
}
