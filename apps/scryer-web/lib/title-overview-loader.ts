import type { Client, CombinedError } from "urql";

import { extractDownloadFeedbackWarning } from "@/lib/graphql/download-feedback-timeout";
import type { Facet } from "@/lib/types";
import type { DownloadQueueItem } from "@/lib/types/download-queue";

import {
  titleBySlugQuery,
  titleOverviewDownloadFeedbackQuery,
  titleOverviewNativeQuery,
} from "@/lib/graphql/queries";

export type TitleOverviewNativeSnapshot<TTitle, TDiagnostics, TEvent, TBlocklist, TSubtitle> = {
  title: TTitle | null;
  acquisitionDiagnostics: TDiagnostics | null;
  titleHistory: TEvent[];
  titleReleaseBlocklist: TBlocklist[];
  externalSubtitles: TSubtitle[];
  hasDownloadClients: boolean;
};

export type TitleOverviewDownloadFeedbackSnapshot = {
  downloadQueueItems: DownloadQueueItem[];
  completedDownloadQueueItems: DownloadQueueItem[];
  downloadFeedbackWarning: string | null;
};

export type TitleOverviewSnapshot<TTitle, TDiagnostics, TEvent, TBlocklist, TSubtitle> =
  TitleOverviewNativeSnapshot<TTitle, TDiagnostics, TEvent, TBlocklist, TSubtitle>
  & TitleOverviewDownloadFeedbackSnapshot;

export type ResolvedTitleOverviewTarget = {
  id: string;
  slug: string | null;
  libraryId: string | null;
  librarySlug: string | null;
};

function graphQlErrorAlias(
  error: CombinedError["graphQLErrors"][number],
): string | null {
  if (!Array.isArray(error.path) || error.path.length === 0) {
    return null;
  }

  return typeof error.path[0] === "string" ? error.path[0] : null;
}

function isTitleOverviewPartialOverviewError(
  error: CombinedError,
  data: unknown,
): boolean {
  if (error.networkError || !data || error.graphQLErrors.length === 0) {
    return false;
  }

  return error.graphQLErrors.every(
    (graphQlError) => {
      const alias = graphQlErrorAlias(graphQlError);
      return alias === "titleHistory" || alias === "setupStatus";
    },
  );
}

// Canonical base loader for title overview pages. Overview containers may
// derive view-specific state locally, but should not duplicate the underlying
// network-only title detail fetch and normalization.
export async function fetchTitleOverviewNativeSnapshot<
  TTitle,
  TDiagnostics = unknown,
  TEvent = unknown,
  TBlocklist = unknown,
  TSubtitle = unknown,
>(
  client: Client,
  titleId: string,
  blocklistLimit: number,
  queryDocument: string = titleOverviewNativeQuery,
) : Promise<TitleOverviewNativeSnapshot<TTitle, TDiagnostics, TEvent, TBlocklist, TSubtitle>> {
  const { data, error } = await client
    .query(
      queryDocument,
      { id: titleId, blocklistLimit },
      { requestPolicy: "network-only" },
    )
    .toPromise();

  if (error && !isTitleOverviewPartialOverviewError(error, data)) {
    throw error;
  }

  return {
    title: (data?.title ?? null) as TTitle | null,
    acquisitionDiagnostics: (data?.titleAcquisitionDiagnostics ?? null) as TDiagnostics | null,
    titleHistory: (data?.titleHistory?.records ?? []) as TEvent[],
    titleReleaseBlocklist: (data?.titleReleaseBlocklist ?? []) as TBlocklist[],
    externalSubtitles: (data?.externalSubtitles ?? []) as TSubtitle[],
    hasDownloadClients: data?.setupStatus?.hasDownloadClients !== false,
  };
}

export function createEmptyTitleOverviewDownloadFeedbackSnapshot(): TitleOverviewDownloadFeedbackSnapshot {
  return {
    downloadQueueItems: [],
    completedDownloadQueueItems: [],
    downloadFeedbackWarning: null,
  };
}

export async function fetchTitleOverviewDownloadFeedbackSnapshot(
  client: Client,
  titleId: string,
): Promise<TitleOverviewDownloadFeedbackSnapshot> {
  const { data, error } = await client
    .query(
      titleOverviewDownloadFeedbackQuery,
      { id: titleId },
      { requestPolicy: "network-only" },
    )
    .toPromise();

  let downloadFeedbackWarning: string | null = null;
  if (error) {
    if (error.networkError || !data) {
      throw error;
    }

    downloadFeedbackWarning = extractDownloadFeedbackWarning(error.graphQLErrors, [
      "downloadQueueItems",
      "completedDownloadQueueItems",
    ]);
    if (!downloadFeedbackWarning) {
      throw error;
    }
  }

  return {
    downloadQueueItems: (data?.downloadQueueItems ?? []) as DownloadQueueItem[],
    completedDownloadQueueItems: (data?.completedDownloadQueueItems ?? []) as DownloadQueueItem[],
    downloadFeedbackWarning,
  };
}

export async function resolveTitleOverviewTargetBySlug(
  client: Client,
  facet: Facet,
  librarySlug: string,
  slug: string,
): Promise<ResolvedTitleOverviewTarget | null> {
  const normalizedLibrarySlug = librarySlug.trim();
  const normalizedSlug = slug.trim();
  if (!normalizedLibrarySlug || !normalizedSlug) {
    return null;
  }

  const { data, error } = await client
    .query(
      titleBySlugQuery,
      { facet, librarySlug: normalizedLibrarySlug, slug: normalizedSlug },
      { requestPolicy: "network-only" },
    )
    .toPromise();

  if (error) {
    throw error;
  }

  const title = data?.titleBySlug;
  if (!title?.id) {
    return null;
  }

  return {
    id: String(title.id),
    slug: typeof title.slug === "string" && title.slug.trim().length > 0
      ? title.slug.trim()
      : null,
    libraryId: typeof title.libraryId === "string" && title.libraryId.trim().length > 0
      ? title.libraryId.trim()
      : null,
    librarySlug: typeof title.librarySlug === "string" && title.librarySlug.trim().length > 0
      ? title.librarySlug.trim()
      : null,
  };
}
