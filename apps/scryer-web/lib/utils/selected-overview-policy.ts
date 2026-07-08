export type SelectedOverviewDetailOwner = "panel" | "native-series-overview";

export function selectedOverviewDetailOwner(
  view: string,
): SelectedOverviewDetailOwner {
  return view === "movies" ? "panel" : "native-series-overview";
}

export function selectedOverviewUsesPanelDetail(view: string): boolean {
  return selectedOverviewDetailOwner(view) === "panel";
}

export function selectedOverviewNativeTitleId(
  view: string,
  selectedOverviewTitleId: string | null,
): string | null {
  return selectedOverviewUsesPanelDetail(view) ? null : selectedOverviewTitleId;
}
