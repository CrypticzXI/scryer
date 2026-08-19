import { Users } from "lucide-react";

import { HorizontalRail } from "@/components/common/horizontal-scroll-fade";
import {
  TitleWorkspaceSectionCard,
  TitleWorkspaceSectionHeader,
} from "@/components/views/media-content/title-workspace-primitives";
import { useTranslate } from "@/lib/context/translate-context";
import type { TitleCreditRecord } from "@/lib/types/titles";
import {
  titleCastCreditCharacter,
  titleCastCreditEpisodeCount,
  titleCastCreditKey,
  titleCastCredits,
} from "@/lib/utils/title-cast";

/**
 * `"panel"` matches the series-overview page chrome, `"workspace"` the title
 * context panel's section cards. The rail itself is identical either way.
 */
export type TitleCastStripVariant = "panel" | "workspace";

type Props = {
  credits?: TitleCreditRecord[] | null;
  variant?: TitleCastStripVariant;
};

/**
 * Top-billed cast for a title, rendered from the cached credits that ride the
 * overview snapshot. The server filters to cast kinds, orders by billing rank,
 * and caps the list, so this renders the response order verbatim.
 *
 * Cards are deliberately non-interactive: there are no person pages yet.
 */
export function TitleCastStrip({ credits, variant = "panel" }: Props) {
  const t = useTranslate();
  const cast = titleCastCredits(credits);

  if (cast.length === 0) {
    return null;
  }

  const heading = t("title.topBilledCast");
  const cards = (
    <HorizontalRail
      className={
        variant === "workspace"
          ? "flex gap-[11px] overflow-x-auto pb-1"
          : "flex gap-3 overflow-x-auto pb-1"
      }
    >
      {cast.map((credit, index) => (
        <TitleCastCard
          key={titleCastCreditKey(credit, index)}
          credit={credit}
          episodeLabel={castCreditEpisodeLabel(credit, t)}
        />
      ))}
    </HorizontalRail>
  );

  if (variant === "workspace") {
    return (
      <TitleWorkspaceSectionCard className="rounded-[14px] bg-[var(--scry-surf)]">
        <TitleWorkspaceSectionHeader icon={Users} title={heading} />
        {cards}
      </TitleWorkspaceSectionCard>
    );
  }

  return (
    <section className="space-y-3 rounded-lg border border-border/70 bg-card/60 p-4">
      <div className="flex items-center gap-2">
        <span className="flex size-8 items-center justify-center rounded-lg bg-primary/15 text-primary">
          <Users className="h-4 w-4" />
        </span>
        <h2 className="text-sm font-semibold uppercase tracking-[0.08em] text-muted-foreground">
          {heading}
        </h2>
      </div>
      {cards}
    </section>
  );
}

function castCreditEpisodeLabel(
  credit: TitleCreditRecord,
  t: ReturnType<typeof useTranslate>,
): string | null {
  const count = titleCastCreditEpisodeCount(credit);
  if (count === null) {
    return null;
  }
  return t(count === 1 ? "title.episodeCountOne" : "title.episodeCountOther", {
    count,
  });
}

function TitleCastCard({
  credit,
  episodeLabel,
}: {
  credit: TitleCreditRecord;
  episodeLabel: string | null;
}) {
  const character = titleCastCreditCharacter(credit);
  // The server hands back the `w185` portrait variant, which is already the
  // card size; no re-varianting needed.
  const portraitUrl = credit.personImageUrl ?? null;

  return (
    <div className="w-24 shrink-0">
      <div className="aspect-[2/3] w-full overflow-hidden rounded-[10px] border border-border/60 bg-muted">
        {portraitUrl ? (
          <img
            src={portraitUrl}
            alt=""
            loading="lazy"
            decoding="async"
            className="h-full w-full object-cover"
          />
        ) : (
          <div className="flex h-full w-full items-center justify-center text-muted-foreground">
            <Users className="h-5 w-5" aria-hidden="true" />
          </div>
        )}
      </div>
      <p
        className="mt-1.5 truncate text-[12px] font-semibold text-foreground"
        title={credit.personName}
      >
        {credit.personName}
      </p>
      {character ? (
        <p className="truncate text-[11px] text-muted-foreground" title={character}>
          {character}
        </p>
      ) : null}
      {episodeLabel ? (
        <p className="truncate text-[11px] text-muted-foreground">
          {episodeLabel}
        </p>
      ) : null}
    </div>
  );
}
