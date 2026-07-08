export type SelectedSidePanelOwner = "movie-record" | "series-container";

export function selectedSidePanelOwner(view: string): SelectedSidePanelOwner {
  return view === "movies" ? "movie-record" : "series-container";
}

export function selectedOverviewUsesMovieRecord(view: string): boolean {
  return selectedSidePanelOwner(view) === "movie-record";
}

export function selectedSeriesSidePanelTitleId(
  view: string,
  selectedOverviewTitleId: string | null,
): string | null {
  return selectedOverviewUsesMovieRecord(view) ? null : selectedOverviewTitleId;
}
