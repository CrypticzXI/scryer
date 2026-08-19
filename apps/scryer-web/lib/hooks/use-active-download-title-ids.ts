import { useMemo } from "react";

import { useDownloadQueue } from "@/lib/hooks/use-download-queue";
import { collectActiveDownloadTitleIds } from "@/lib/utils/catalog-download-activity";

const EMPTY_TITLE_IDS: ReadonlySet<string> = new Set<string>();
// A title id never contains a newline, so joining on one keeps the membership
// key collision-proof.
const TITLE_ID_KEY_SEPARATOR = "\n";

// The catalog indicator is decorative — a queue hiccup must not raise a toast
// over the catalog, so swallow the error instead of letting it fall through to
// the global status channel.
const ignoreQueueError = () => {};

/**
 * Titles with live, pending download work, derived from the shared download
 * queue feed (the same subscription the Activity view uses, with import
 * activity folded in so post-download import work still reads as "pending").
 *
 * Returns a referentially stable set while the membership is unchanged, so the
 * memoized catalog renderers don't re-render on every queue heartbeat.
 */
export function useActiveDownloadTitleIds({
  enabled,
}: {
  enabled: boolean;
}): ReadonlySet<string> {
  const { queueItems } = useDownloadQueue({
    enabled,
    includeAllActivity: true,
    includeHistoryOnly: false,
    includeImportActivity: true,
    activityFilter: "ALL",
    onErrorStatus: ignoreQueueError,
  });

  const activeTitleIdsKey = useMemo(
    () =>
      [...collectActiveDownloadTitleIds(queueItems)]
        .sort()
        .join(TITLE_ID_KEY_SEPARATOR),
    [queueItems],
  );

  return useMemo<ReadonlySet<string>>(
    () =>
      activeTitleIdsKey.length === 0
        ? EMPTY_TITLE_IDS
        : new Set(activeTitleIdsKey.split(TITLE_ID_KEY_SEPARATOR)),
    [activeTitleIdsKey],
  );
}
