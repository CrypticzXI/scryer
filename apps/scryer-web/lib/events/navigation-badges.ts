export const NAVIGATION_BADGES_REFRESH_EVENT = "scryer:navigationBadgesRefresh";

export type NavigationBadgesRefreshDetail = {
  delta?: number;
};

export function dispatchNavigationBadgesRefresh(
  detail?: NavigationBadgesRefreshDetail,
) {
  if (typeof window === "undefined") {
    return;
  }

  window.dispatchEvent(
    new CustomEvent(
      NAVIGATION_BADGES_REFRESH_EVENT,
      detail ? { detail } : undefined,
    ),
  );
}
