const LINKED_REQUEST_EXTERNAL_ID_SOURCES = new Set([
  "imdb",
  "tvdb",
  "tmdb",
  "mal",
  "anilist",
  "anidb",
]);

export type RequestExternalIdChip = {
  source: string;
  value: string;
};

export function getRawRequestExternalIds(
  externalIds: Array<{ source: string; value: string }>,
): RequestExternalIdChip[] {
  const seen = new Set<string>();
  const rawIds: RequestExternalIdChip[] = [];

  for (const externalId of externalIds) {
    const source = externalId.source.trim().toLowerCase();
    const value = externalId.value.trim();
    const key = `${source}:${value}`;
    if (!source || !value || LINKED_REQUEST_EXTERNAL_ID_SOURCES.has(source) || seen.has(key)) {
      continue;
    }
    seen.add(key);
    rawIds.push({ source, value });
  }

  return rawIds;
}

export function formatExternalIdSourceLabel(source: string): string {
  return source.replace(/[-_]+/g, " ").toUpperCase();
}
