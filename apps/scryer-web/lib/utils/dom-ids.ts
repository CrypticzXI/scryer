export function selectorToken(value: string | number): string {
  const normalized = String(value)
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/(^-|-$)+/g, "");

  return normalized || "item";
}

type MetadataSearchSelectorInput = {
  name: string;
  imdbId?: string | null;
  tvdbId?: string | number | null;
};

export function metadataSearchSelectorParts(
  result: MetadataSearchSelectorInput,
): string[] {
  const imdbId = result.imdbId?.trim();
  if (imdbId) {
    return ["imdb", imdbId];
  }

  const tvdbId = String(result.tvdbId ?? "").trim();
  if (tvdbId) {
    return ["tvdb", tvdbId];
  }

  return ["name", result.name];
}

export function globalSearchMetadataResultId(
  facet: string,
  result: MetadataSearchSelectorInput,
): string {
  return selectorId(
    "global-search-metadata-result",
    facet,
    ...metadataSearchSelectorParts(result),
  );
}

export function titleOverviewRowId(titleId: string): string {
  return selectorId("title-overview-row", titleId);
}

export function titleOverviewSearchButtonId(titleId: string): string {
  return selectorId("title-overview-search", titleId);
}

export function globalSearchConfigureAddId(
  facet: string,
  result: MetadataSearchSelectorInput,
): string {
  return selectorId(
    "global-search-configure-add",
    facet,
    ...metadataSearchSelectorParts(result),
  );
}

export function selectorId(
  ...parts: Array<string | number | false | null | undefined>
): string {
  return parts
    .filter(
      (part): part is string | number =>
        part !== false &&
        part !== null &&
        part !== undefined &&
        String(part).trim().length > 0,
    )
    .map((part) => selectorToken(part))
    .join("-");
}
