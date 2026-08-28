import { useEffect, useRef } from "react";
import { useClient } from "urql";

import { showCatalogAddToast } from "@/components/root/catalog-add-toast";
import { useTranslate } from "@/lib/context/translate-context";
import { titleAutocompleteSelectionQuery } from "@/lib/graphql/queries";
import { useActivityEventStream } from "@/lib/hooks/use-activity-event-stream";
import type { TitleRecord } from "@/lib/types";

const TITLE_ACTIVITY_KINDS = new Set([
  "acquisition_candidate_accepted",
  "movie_downloaded",
  "series_episode_imported",
]);
const REPLAY_GRACE_MS = 2_000;

export function GrabbedReleaseToastListener() {
  const client = useClient();
  const t = useTranslate();
  const mountedAtRef = useRef<number | null>(null);
  const shownEventIdsRef = useRef(new Set<string>());

  useEffect(() => {
    mountedAtRef.current = Date.now();
  }, []);

  useActivityEventStream({
    kinds: TITLE_ACTIVITY_KINDS,
    onEvent(activity) {
      const titleId = activity.titleId?.trim();
      const occurredAt = Date.parse(activity.occurredAt ?? "");
      const mountedAt = mountedAtRef.current;
      if (
        mountedAt === null ||
        !titleId ||
        !Number.isFinite(occurredAt) ||
        occurredAt < mountedAt - REPLAY_GRACE_MS ||
        shownEventIdsRef.current.has(activity.id)
      ) {
        return;
      }
      shownEventIdsRef.current.add(activity.id);

      void client
        .query<{ title?: TitleRecord | null }>(titleAutocompleteSelectionQuery, {
          id: titleId,
        })
        .toPromise()
        .then(({ data, error }) => {
          const title = data?.title;
          if (error || !title) {
            return;
          }
          showCatalogAddToast({
            titleName: title.name,
            year: title.year,
            posterUrl: title.posterUrl,
            headline:
              activity.kind === "acquisition_candidate_accepted"
                ? t("toast.releaseGrabbed")
                : t("toast.titleImported"),
            note: activity.message,
            posterEmptyLabel: t("label.noArt"),
            viewLabel: t("toast.viewInCatalog"),
            dismissLabel: t("label.dismiss"),
          });
        });
    },
  });

  return null;
}
