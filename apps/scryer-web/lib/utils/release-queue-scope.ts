import type { Release, ReleaseQueueScope } from "@/lib/types/releases";

export type QueueDownloadScopeInput =
  | { episode: string }
  | { episodeSet: string[] }
  | { collection: string }
  | { seriesMovie: string }
  | { title: boolean }
  | { orphan: boolean };

function queueScopeToInput(scope: ReleaseQueueScope): QueueDownloadScopeInput | null {
  switch (scope.kind) {
    case "episode":
      return scope.episodeId ? { episode: scope.episodeId } : null;
    case "episode_set":
      return scope.episodeIds.length > 0 ? { episodeSet: scope.episodeIds } : null;
    case "collection":
      return scope.collectionId ? { collection: scope.collectionId } : null;
    case "series_movie":
      return scope.seriesMovieLinkId ? { seriesMovie: scope.seriesMovieLinkId } : null;
    case "title":
      return { title: true };
    case "orphan":
      return { orphan: true };
    default:
      return null;
  }
}

export function releaseQueueScopeInput(
  release: Pick<Release, "queueScope">,
  fallback: QueueDownloadScopeInput,
): QueueDownloadScopeInput {
  const candidateScope = release.queueScope ? queueScopeToInput(release.queueScope) : null;
  return candidateScope ?? fallback;
}

export function releaseSupportsAdditionalFileQueue(
  release: Pick<Release, "queueScope">,
  titleFacet: string | null | undefined,
): boolean {
  const scope = release.queueScope ? queueScopeToInput(release.queueScope) : null;
  if (!scope) {
    return false;
  }
  if ("episode" in scope) {
    return true;
  }
  if ("title" in scope) {
    return titleFacet === "movie";
  }
  return false;
}
