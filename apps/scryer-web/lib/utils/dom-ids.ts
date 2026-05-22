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

export function titleOverviewOpenButtonId(titleId: string): string {
  return selectorId("title-overview-open", titleId);
}

function normalizedEpisodeSelectorKey(
  facet: string,
  seasonNumber: string | number | null | undefined,
  episodeNumber: string | number | null | undefined,
  absoluteNumber: string | number | null | undefined,
): string {
  const season = Number.parseInt(String(seasonNumber ?? "").trim(), 10);
  const episode = Number.parseInt(String(episodeNumber ?? "").trim(), 10);
  if (Number.isFinite(season) && season > 0 && Number.isFinite(episode) && episode > 0) {
    return selectorId(facet, `s${String(season).padStart(2, "0")}e${String(episode).padStart(2, "0")}`);
  }

  const absolute = Number.parseInt(String(absoluteNumber ?? "").trim(), 10);
  if (Number.isFinite(absolute) && absolute > 0) {
    return selectorId(facet, `abs${String(absolute).padStart(3, "0")}`);
  }

  return selectorId(facet, "episode");
}

export function seriesOverviewEpisodeRowId(
  facet: string,
  seasonNumber: string | number | null | undefined,
  episodeNumber: string | number | null | undefined,
  absoluteNumber: string | number | null | undefined,
): string {
  return selectorId(
    "series-overview-episode",
    normalizedEpisodeSelectorKey(facet, seasonNumber, episodeNumber, absoluteNumber),
  );
}

export function seriesOverviewEpisodeAutoSearchId(
  facet: string,
  seasonNumber: string | number | null | undefined,
  episodeNumber: string | number | null | undefined,
  absoluteNumber: string | number | null | undefined,
): string {
  return selectorId(
    "series-overview-episode-auto-search",
    normalizedEpisodeSelectorKey(facet, seasonNumber, episodeNumber, absoluteNumber),
  );
}

export function seriesOverviewEpisodeInteractiveSearchId(
  facet: string,
  seasonNumber: string | number | null | undefined,
  episodeNumber: string | number | null | undefined,
  absoluteNumber: string | number | null | undefined,
): string {
  return selectorId(
    "series-overview-episode-interactive-search",
    normalizedEpisodeSelectorKey(facet, seasonNumber, episodeNumber, absoluteNumber),
  );
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
