export type PendingImportCounts = {
  movie: number;
  series: number;
  anime: number;
};

export type PendingImportStatus = "pending" | "ignored";

export type PendingImportSearchAttempt = {
  query: string;
  resultCount: number;
  topResults: string[];
  summary: string;
};

export type PendingImportItem = {
  id: string;
  facet: "movie" | "series" | "anime";
  status: PendingImportStatus;
  displayName: string;
  path: string;
  folderPath?: string | null;
  query: string;
  yearHint?: number | null;
  reason: string;
  searchAttempts: PendingImportSearchAttempt[];
};

export type PendingImportConnection = {
  total: number;
  items: PendingImportItem[];
};

export type ResolvePendingImportResult = {
  title: {
    id: string;
    name: string;
    facet: string;
    monitored: boolean;
  };
  created: boolean;
  libraryScan: {
    scanned: number;
    matched: number;
    imported: number;
    skipped: number;
    unmatched: number;
  };
};

export function pendingImportCountForView(
  counts: PendingImportCounts | null | undefined,
  view: string,
): number {
  if (!counts) {
    return 0;
  }

  switch (view) {
    case "movies":
      return counts.movie;
    case "series":
      return counts.series;
    case "anime":
      return counts.anime;
    default:
      return 0;
  }
}

export function hasImportItemsForView(
  pendingCounts: PendingImportCounts | null | undefined,
  ignoredCounts: PendingImportCounts | null | undefined,
  view: string,
): boolean {
  return (
    pendingImportCountForView(pendingCounts, view) > 0 ||
    pendingImportCountForView(ignoredCounts, view) > 0
  );
}

export function pendingImportFacetValueForView(
  view: string,
): "movie" | "series" | "anime" {
  switch (view) {
    case "movies":
      return "movie";
    case "series":
      return "series";
    case "anime":
      return "anime";
    default:
      throw new Error(`unsupported pending import view: ${view}`);
  }
}
