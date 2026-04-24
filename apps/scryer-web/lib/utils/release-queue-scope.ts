import type { Release, ReleaseQueueScope } from "@/lib/types/releases";

export type QueueDownloadScopeInput =
  | { episode: string }
  | { episodeSet: string[] }
  | { collection: string }
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
