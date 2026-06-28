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

export type TitleCatalogQueryOptions = {
  facet: string;
  libraryIds: string[];
  query: string;
  filters: TitleCatalogQuickFilters;
  sort: TitleCatalogSortStateLike;
  limit: number;
  offset: number;
};

export const EMPTY_TITLE_QUICK_FILTERS: TitleCatalogQuickFilters = {
  monitored: false,
  unmonitored: false,
  continuing: false,
  ended: false,
};

export function titleCatalogSortInput(sort: TitleCatalogSortStateLike) {
  const key =
    sort.key === "name"
      ? "title"
      : sort.key === "library"
        ? "library"
        : sort.key === "monitored"
          ? "monitored"
          : sort.key === "quality"
            ? "quality"
            : sort.key === "episodes"
              ? "episodes"
              : sort.key === "status"
                ? "status"
                : sort.key === "added"
                  ? "added"
                  : "size";

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

export function titleCatalogQueryKey({
  facet,
  query,
  libraryIds,
  filters,
  sort,
}: Pick<
  TitleCatalogQueryOptions,
  "facet" | "query" | "libraryIds" | "filters" | "sort"
>) {
  return JSON.stringify({
    facet,
    query: query.trim(),
    libraryIds: [...libraryIds].sort(),
    filter: titleCatalogFilterInput(filters),
    sort: titleCatalogSortInput(sort),
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
