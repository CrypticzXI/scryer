import { Sparkles } from "lucide-react";

import { TitleCard } from "@/components/title-card";
import { useTranslate } from "@/lib/context/translate-context";
import type { CatalogDiscoveryItem } from "@/lib/types/discovery";
import { discoveryItemDisplayTitle } from "@/lib/utils/discovery-display";
import { selectPosterVariantUrl } from "@/lib/utils/poster-images";

type Props = {
  items: CatalogDiscoveryItem[];
  fallbackYearLabel: string;
};

export function TitleMoreLikeThisStrip({ items, fallbackYearLabel }: Props) {
  const t = useTranslate();

  if (items.length === 0) {
    return null;
  }

  return (
    <section className="space-y-3 rounded-lg border border-border/70 bg-card/60 p-4">
      <div className="flex items-center gap-2">
        <span className="flex size-8 items-center justify-center rounded-lg bg-primary/15 text-primary">
          <Sparkles className="h-4 w-4" />
        </span>
        <h2 className="text-sm font-semibold uppercase tracking-[0.08em] text-muted-foreground">
          {t("title.contextMoreLikeThis")}
        </h2>
      </div>
      <div className="flex gap-3 overflow-x-auto pb-1">
        {items.map((item) => {
          const year =
            typeof item.year === "number" && Number.isFinite(item.year)
              ? String(item.year)
              : fallbackYearLabel;
          return (
            <div key={item.id} className="w-28 shrink-0">
              <TitleCard
                title={discoveryItemDisplayTitle(item)}
                year={year}
                posterUrl={selectPosterVariantUrl(item.posterUrl, "w250")}
                compact
              />
            </div>
          );
        })}
      </div>
    </section>
  );
}
