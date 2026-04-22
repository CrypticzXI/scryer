import type { Client } from "urql";

import type { Facet } from "@/lib/types";

import { titleBySlugQuery, titleOverviewInitQuery } from "@/lib/graphql/queries";

export type TitleOverviewSnapshot<TTitle, TDiagnostics, TEvent, TBlocklist, TSubtitle> = {
  title: TTitle | null;
  acquisitionDiagnostics: TDiagnostics | null;
  titleEvents: TEvent[];
  titleReleaseBlocklist: TBlocklist[];
  subtitleDownloads: TSubtitle[];
  hasDownloadClients: boolean;
};

export type ResolvedTitleOverviewTarget = {
  id: string;
  slug: string | null;
};

// Canonical base loader for title overview pages. Overview containers may
// derive view-specific state locally, but should not duplicate the underlying
// network-only title detail fetch and normalization.
export async function fetchTitleOverviewSnapshot<
  TTitle,
  TDiagnostics = unknown,
  TEvent = unknown,
  TBlocklist = unknown,
  TSubtitle = unknown,
>(
  client: Client,
  titleId: string,
  blocklistLimit: number,
): Promise<TitleOverviewSnapshot<TTitle, TDiagnostics, TEvent, TBlocklist, TSubtitle>> {
  const { data, error } = await client
    .query(
      titleOverviewInitQuery,
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
    subtitleDownloads: (data?.subtitleDownloads ?? []) as TSubtitle[],
    hasDownloadClients: data?.setupStatus?.hasDownloadClients === true,
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
