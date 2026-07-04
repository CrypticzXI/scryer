export type TitleCatalogQuickFilters = {
  monitored: boolean;
  unmonitored: boolean;
  continuing: boolean;
  ended: boolean;
};

export type TitleCatalogSortStateLike = {
  key: string;
  direction: string;
};

export type TitleCatalogProjection = {
  library: boolean;
  quality: boolean;
  size: boolean;
  episodes: boolean;
  runtime: boolean;
  root: boolean;
  ratings: boolean;
  movieMedia: boolean;
  popularity: boolean;
};

export type TitleCatalogQueryOptions = {
  facet: string;
  libraryIds: string[];
  query: string;
  filters: TitleCatalogQuickFilters;
  sort: TitleCatalogSortStateLike;
  projection?: TitleCatalogProjection;
  limit: number;
  offset: number;
};

export const EMPTY_TITLE_QUICK_FILTERS: TitleCatalogQuickFilters = {
  monitored: false,
  unmonitored: false,
  continuing: false,
  ended: false,
};

const EMPTY_TITLE_CATALOG_PROJECTION: TitleCatalogProjection = {
  library: false,
  quality: false,
  size: false,
  episodes: false,
  runtime: false,
  root: false,
  ratings: false,
  movieMedia: false,
  popularity: false,
};

const TITLE_CATALOG_SORT_KEYS: Record<string, string> = {
  name: "title",
  library: "library",
  monitored: "monitored",
  quality: "quality",
  episodes: "episodes",
  status: "status",
  added: "added",
  size: "size",
  year: "year",
  runtime: "runtime",
  root: "root",
  popularity: "popularity",
  resolution: "media_resolution",
  hdr: "media_hdr",
  audioCodec: "media_audio_codec",
  ratingScryer: "rating_scryer",
  ratingImdb: "rating_imdb",
  ratingRottenTomatoes: "rating_rotten_tomatoes",
  ratingPopcornmeter: "rating_popcornmeter",
  ratingMetacritic: "rating_metacritic",
  ratingMetacriticUser: "rating_metacritic_user",
  ratingLetterboxd: "rating_letterboxd",
  ratingTmdb: "rating_tmdb",
  ratingTvdb: "rating_tvdb",
  ratingTrakt: "rating_trakt",
  ratingMyanimelist: "rating_myanimelist",
  ratingAnilist: "rating_anilist",
  ratingAnidb: "rating_anidb",
  ratingMdblist: "rating_mdblist",
};

const SHARED_RATING_COLUMN_KEYS = new Set([
  "ratingScryer",
  "ratingImdb",
  "ratingRottenTomatoes",
  "ratingPopcornmeter",
  "ratingMetacritic",
  "ratingMetacriticUser",
  "ratingLetterboxd",
  "ratingTmdb",
  "ratingTvdb",
  "ratingTrakt",
  "ratingMdblist",
]);

const ANIME_RATING_COLUMN_KEYS = new Set([
  "ratingScryer",
  "ratingImdb",
  "ratingTmdb",
  "ratingTvdb",
  "ratingTrakt",
  "ratingMyanimelist",
  "ratingAnilist",
  "ratingAnidb",
  "ratingMdblist",
]);

function normalizedFacet(facet: string) {
  return facet === "movie" || facet === "series" || facet === "anime"
    ? facet
    : null;
}

export function titleCatalogSortInput(sort: TitleCatalogSortStateLike) {
  const key = TITLE_CATALOG_SORT_KEYS[sort.key] ?? "size";

  return {
    key,
    direction: sort.direction,
  };
}

export function titleCatalogFilterInput(filters: TitleCatalogQuickFilters) {
  const monitored =
    filters.monitored === filters.unmonitored
      ? null
      : filters.monitored
        ? true
        : false;
  const contentStatuses = [
    filters.continuing ? "continuing" : null,
    filters.ended ? "ended" : null,
  ].filter((value): value is string => Boolean(value));

  if (monitored === null && contentStatuses.length === 0) {
    return null;
  }

  return {
    monitored,
    contentStatuses,
  };
}

export function titleCatalogProjectionSignature(
  projection: TitleCatalogProjection | undefined,
) {
  const normalized = projection ?? EMPTY_TITLE_CATALOG_PROJECTION;
  return [
    normalized.library && "library",
    normalized.quality && "quality",
    normalized.size && "size",
    normalized.episodes && "episodes",
    normalized.runtime && "runtime",
    normalized.root && "root",
    normalized.ratings && "ratings",
    normalized.movieMedia && "movieMedia",
    normalized.popularity && "popularity",
  ]
    .filter(Boolean)
    .join(":");
}

export function titleCatalogProjectionForTable({
  facet,
  visibleColumns,
  sort,
}: {
  facet: string;
  visibleColumns: Partial<Record<string, boolean>>;
  sort: TitleCatalogSortStateLike;
}): TitleCatalogProjection {
  const next = { ...EMPTY_TITLE_CATALOG_PROJECTION };
  const activeFacet = normalizedFacet(facet);
  const supportedRatingColumnKeys =
    activeFacet === "anime" ? ANIME_RATING_COLUMN_KEYS : SHARED_RATING_COLUMN_KEYS;
  const selectedOrSorted = (key: string) =>
    visibleColumns[key] === true || sort.key === key;
  const anyRatingSelectedOrSorted = Object.keys(visibleColumns).some(
    (key) => supportedRatingColumnKeys.has(key) && visibleColumns[key] === true,
  ) || supportedRatingColumnKeys.has(sort.key);

  next.library = selectedOrSorted("library");
  next.quality = selectedOrSorted("quality");
  next.size = selectedOrSorted("size");
  next.episodes = activeFacet !== "movie" && selectedOrSorted("episodes");
  next.runtime = selectedOrSorted("runtime");
  next.root = selectedOrSorted("root");
  next.ratings = anyRatingSelectedOrSorted;
  next.movieMedia =
    activeFacet === "movie" &&
    (selectedOrSorted("resolution") ||
      selectedOrSorted("hdr") ||
      selectedOrSorted("audioCodec"));
  next.popularity = activeFacet === "movie" && selectedOrSorted("popularity");

  return next;
}

export function titleCatalogQueryKey({
  facet,
  query,
  libraryIds,
  filters,
  sort,
  projection,
}: Pick<
  TitleCatalogQueryOptions,
  "facet" | "query" | "libraryIds" | "filters" | "sort" | "projection"
>) {
  return JSON.stringify({
    facet,
    query: query.trim(),
    libraryIds: [...libraryIds].sort(),
    filter: titleCatalogFilterInput(filters),
    sort: titleCatalogSortInput(sort),
    projection: titleCatalogProjectionSignature(projection),
  });
}

export function buildTitleCatalogQueryVariables({
  facet,
  libraryIds,
  query,
  filters,
  sort,
  limit,
  offset,
}: TitleCatalogQueryOptions) {
  return {
    facet,
    libraryIds: libraryIds.length > 0 ? libraryIds : null,
    query: query.trim() || null,
    filter: titleCatalogFilterInput(filters),
    sort: titleCatalogSortInput(sort),
    limit,
    offset,
  };
}
