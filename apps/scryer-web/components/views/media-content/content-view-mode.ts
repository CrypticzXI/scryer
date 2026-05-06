export type ContentViewMode = "compact" | "poster-table" | "poster";

export const CONTENT_VIEW_MODE_STORAGE_KEY = "scryer:content-view-mode";

export function readStoredContentViewMode(): ContentViewMode {
  if (typeof window === "undefined") {
    return "poster";
  }

  try {
    const stored = window.localStorage.getItem(CONTENT_VIEW_MODE_STORAGE_KEY);
    switch (stored) {
      case "compact":
        return "compact";
      case "poster":
        return "poster";
      case "poster-table":
      case "table":
        return "poster-table";
      default:
        return "poster";
    }
  } catch {
    return "poster";
  }
}

export function writeStoredContentViewMode(mode: ContentViewMode) {
  if (typeof window === "undefined") {
    return;
  }

  try {
    window.localStorage.setItem(CONTENT_VIEW_MODE_STORAGE_KEY, mode);
  } catch {
    // Ignore persistence failures.
  }
}
