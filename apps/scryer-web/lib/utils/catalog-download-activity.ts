import type { DownloadQueueItem } from "@/lib/types";
import { normalizeQueueState } from "./download-queue.ts";

/**
 * The slice of a queue item the catalog download indicator cares about. Kept as
 * a `Pick` so the predicate stays trivially testable and works for both live
 * subscription payloads and query results.
 */
export type CatalogDownloadActivityInput = Pick<
  DownloadQueueItem,
  "titleId" | "displayState"
>;

/**
 * Display states that mean "work is still pending for this title" — everything
 * from sitting in the client queue through the import finishing.
 *
 * Deliberately excluded: `paused` (user parked it), `import_blocked` /
 * `import_failed` / `failed` / `remove_failed` (needs a human, not progress),
 * `removing` (being torn down), and `completed` / `ignored` (historical).
 */
const PENDING_CATALOG_DOWNLOAD_DISPLAY_STATES: ReadonlySet<string> = new Set([
  "queued",
  "downloading",
  "post_processing",
  "import_pending",
  "importing",
]);

/**
 * True when a queue item represents live, title-linked work that the catalog
 * should surface as a pulsing "Downloading" pill.
 *
 * Items with no linked title can't be attributed to a catalog row, so they
 * never count no matter what state they're in.
 */
export function isPendingCatalogDownloadQueueItem(
  item: CatalogDownloadActivityInput,
): boolean {
  if (!item.titleId || item.titleId.trim().length === 0) {
    return false;
  }

  return PENDING_CATALOG_DOWNLOAD_DISPLAY_STATES.has(
    normalizeQueueState(item.displayState),
  );
}

/**
 * Collapse a queue snapshot into the set of title ids with pending work.
 * Several queue items for one title collapse to a single entry.
 */
export function collectActiveDownloadTitleIds(
  items: readonly CatalogDownloadActivityInput[],
): Set<string> {
  const titleIds = new Set<string>();
  for (const item of items) {
    if (isPendingCatalogDownloadQueueItem(item)) {
      titleIds.add(item.titleId as string);
    }
  }
  return titleIds;
}
