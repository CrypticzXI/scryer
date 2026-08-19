import * as React from "react";
import { Download } from "lucide-react";

import type { TitleCardCornerBadge } from "@/components/title-card";
import type { Translate } from "@/components/root/types";
import { ActionTooltip } from "@/components/ui/tooltip";
import { useTranslate } from "@/lib/context/translate-context";
import { cn } from "@/lib/utils";

/**
 * Short, already-localized state word shared by every catalog download
 * indicator. Reuses the Activity view's queue vocabulary so the catalog and the
 * queue never disagree about what a state is called.
 */
export function titleDownloadActivityStateLabel(t: Translate): string {
  return t("queue.state.downloading");
}

/** Title-specific accessible label, e.g. "Downloading: Oppenheimer". */
export function titleDownloadActivityLabel(
  t: Translate,
  titleName: string,
): string {
  return `${titleDownloadActivityStateLabel(t)}: ${titleName}`;
}

/**
 * The poster-grid flavour of the same indicator: a pulsing accent corner badge
 * on the card. Kept next to the pill so both surfaces share one label and one
 * visual treatment.
 */
export function titleDownloadActivityCornerBadge(
  t: Translate,
  titleName: string,
): TitleCardCornerBadge {
  return {
    label: titleDownloadActivityStateLabel(t),
    icon: Download,
    tone: "accent",
    title: titleDownloadActivityLabel(t, titleName),
    pulse: true,
  };
}

/**
 * Qualitative "this title has download work in flight" marker for catalog table
 * rows — no percentage, count, or progress bar, just a pulsing accent pill that
 * sits beside the title text so it never needs a column of its own.
 *
 * Expects a `TooltipProvider` ancestor (both catalog tables wrap their body in
 * one).
 */
export const TitleDownloadActivityPill = React.memo(
  function TitleDownloadActivityPill({
    titleName,
    className,
  }: {
    titleName: string;
    className?: string;
  }) {
    const t = useTranslate();
    const accessibleLabel = titleDownloadActivityLabel(t, titleName);

    return (
      <ActionTooltip
        useProvider={false}
        content={accessibleLabel}
        wrapperClassName="shrink-0"
      >
        <span
          data-ui="title-download-activity-pill"
          role="img"
          aria-label={accessibleLabel}
          className={cn(
            "inline-flex shrink-0 animate-pulse items-center gap-1 rounded-[6px] border border-[rgba(var(--scry-accent-rgb),0.42)] bg-[rgba(var(--scry-accent-rgb),0.16)] px-1.5 py-0.5 text-[10px] font-semibold uppercase leading-none tracking-[0.03em] text-[var(--scry-accent-text)] motion-reduce:animate-none",
            className,
          )}
        >
          <Download className="size-2.5 shrink-0" aria-hidden="true" />
          <span className="whitespace-nowrap">
            {titleDownloadActivityStateLabel(t)}
          </span>
        </span>
      </ActionTooltip>
    );
  },
);
