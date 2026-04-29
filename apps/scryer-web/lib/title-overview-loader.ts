import type { Client } from "urql";

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
  titleEvents: TEvent[];
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
};

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
) : Promise<TitleOverviewNativeSnapshot<TTitle, TDiagnostics, TEvent, TBlocklist, TSubtitle>> {
  const { data, error } = await client
    .query(
      titleOverviewNativeQuery,
      { id: titleId, blocklistLimit },
      { requestPolicy: "network-only" },
    )
    .toPromise();

  if (error) {
    throw error;
  }

  return {
    title: (data?.title ?? null) as TTitle | null,
    acquisitionDiagnostics: (data?.titleAcquisitionDiagnostics ?? null) as TDiagnostics | null,
    titleEvents: (data?.titleEvents ?? []) as TEvent[],
    titleReleaseBlocklist: (data?.titleReleaseBlocklist ?? []) as TBlocklist[],
    externalSubtitles: (data?.externalSubtitles ?? []) as TSubtitle[],
    hasDownloadClients: data?.setupStatus?.hasDownloadClients === true,
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
  slug: string,
): Promise<ResolvedTitleOverviewTarget | null> {
  const normalizedSlug = slug.trim();
  if (!normalizedSlug) {
    return null;
  }

  const { data, error } = await client
    .query(
      titleBySlugQuery,
      { facet, slug: normalizedSlug },
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
  };
}
