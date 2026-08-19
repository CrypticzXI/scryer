import type { TitleCreditRecord } from "@/lib/types/titles";

/**
 * Drop credits with nothing to render. The server already filtered by kind,
 * ordered by billing rank, and applied the limit, so the surviving order is the
 * server's — never re-sort here.
 */
export function titleCastCredits(
  credits: TitleCreditRecord[] | null | undefined,
): TitleCreditRecord[] {
  return (credits ?? []).filter(
    (credit) => (credit?.personName ?? "").trim().length > 0,
  );
}

/**
 * React key for a cast card. Person identity is deliberately not exposed by the
 * API, so billing rank plus the rendered name is the most stable key available;
 * the index keeps it unique when a provider bills two people identically.
 */
export function titleCastCreditKey(
  credit: TitleCreditRecord,
  index: number,
): string {
  return `${credit.billingOrder ?? index}-${credit.personName}-${index}`;
}

/**
 * Episode count to show under a cast card, or null when the provider does not
 * count episodes for this title (movies) or reported a meaningless value.
 */
export function titleCastCreditEpisodeCount(
  credit: TitleCreditRecord,
): number | null {
  const count = credit.episodeCount;
  if (typeof count !== "number" || !Number.isFinite(count) || count <= 0) {
    return null;
  }
  return Math.trunc(count);
}

/** Character subline, or null when the provider supplied none. */
export function titleCastCreditCharacter(
  credit: TitleCreditRecord,
): string | null {
  const character = (credit.character ?? "").trim();
  return character.length > 0 ? character : null;
}
