import type { TitleCreditRecord } from "@/lib/types/titles";

/**
 * Cards shown per rail. The overview fetches up to the server's 50-credit clamp
 * so the original/dub split has enough rows on both sides, then each rail caps
 * its own display here.
 */
export const TITLE_CAST_RAIL_DISPLAY_LIMIT = 15;

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
 * Main-rail cast: on-screen performers plus the original Japanese voice cast.
 * TMDB actor rows carry no language; anime titles only have voice_actor rows,
 * so the `ja` filter is what keeps the main rail single-cast.
 */
export function titleCastOriginalCredits(
  credits: TitleCreditRecord[] | null | undefined,
): TitleCreditRecord[] {
  return titleCastCredits(credits)
    .filter(
      (credit) =>
        credit.kind !== "voice_actor" || (credit.language ?? "") === "ja",
    )
    .slice(0, TITLE_CAST_RAIL_DISPLAY_LIMIT);
}

/**
 * Dub-rail cast: voice actors in one non-Japanese dub language. Empty for
 * movies and live-action series, which renders no dub rail at all.
 *
 * `language` is a dub language code (`en`, `de`, ...). Omitting it returns
 * every dub language at once, which is only useful for "is there a dub rail"
 * checks — the rail itself always picks one language.
 */
export function titleCastDubCredits(
  credits: TitleCreditRecord[] | null | undefined,
  language?: string | null,
): TitleCreditRecord[] {
  return titleCastCredits(credits)
    .filter(
      (credit) =>
        credit.kind === "voice_actor" &&
        (credit.language ?? "").length > 0 &&
        credit.language !== "ja" &&
        (!language || credit.language === language),
    )
    .slice(0, TITLE_CAST_RAIL_DISPLAY_LIMIT);
}

/**
 * Dub languages this title actually has credits for, sorted by code so the
 * picker order is stable. SMG only harvests `ja`/`en` today, but its VA config
 * already supports es/fr/de/it/pt/ko/zh — those appear here on their own once
 * the data flows, with no client change.
 */
export function titleCastDubLanguages(
  credits: TitleCreditRecord[] | null | undefined,
): string[] {
  const languages = new Set<string>();
  for (const credit of titleCastCredits(credits)) {
    const language = credit.language ?? "";
    if (credit.kind === "voice_actor" && language.length > 0 && language !== "ja") {
      languages.add(language);
    }
  }
  return [...languages].sort();
}

/**
 * Which dub language the rail opens on: the viewer's prior pick when that
 * language is still present, otherwise English, otherwise the first available.
 */
export function titleCastPreferredDubLanguage(
  languages: string[],
  preferred?: string | null,
): string | null {
  if (preferred && languages.includes(preferred)) {
    return preferred;
  }
  if (languages.includes("en")) {
    return "en";
  }
  return languages[0] ?? null;
}

/**
 * Localized display name for a dub language code, falling back to the raw code
 * for anything the runtime cannot name.
 */
export function titleCastDubLanguageLabel(
  language: string,
  locale?: string,
): string {
  try {
    const names = new Intl.DisplayNames([locale ?? "en"], { type: "language" });
    return names.of(language) ?? language;
  } catch {
    return language;
  }
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
