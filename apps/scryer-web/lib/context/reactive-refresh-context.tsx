import { createContext, useContext } from "react";

import type { TitleOverviewNativeProjection } from "@/lib/graphql/queries";
import type { ImportRecord, TitleRecord } from "@/lib/types";
import type {
  TitleOverviewDownloadFeedbackSnapshot,
  TitleOverviewNativeSnapshot,
} from "@/lib/title-overview-loader";

type ReactiveRefreshErrorHandler = (error: unknown) => void;

export type QueueCatalogTitlesRefreshOptions = {
  facet?: string | null;
  apply: (titles: TitleRecord[]) => void;
  onError?: ReactiveRefreshErrorHandler;
};

export type QueueCatalogTitleRefreshOptions = {
  titleId: string;
  apply: (title: TitleRecord | null, requestEpoch: number) => void;
  onError?: ReactiveRefreshErrorHandler;
};

export type QueueTitleOverviewNativeRefreshOptions<
  TTitle = unknown,
  TDiagnostics = unknown,
  TEvent = unknown,
  TBlocklist = unknown,
  TSubtitle = unknown,
> = {
  titleId: string;
  blocklistLimit: number;
  projection?: TitleOverviewNativeProjection;
  apply: (
    snapshot: TitleOverviewNativeSnapshot<
      TTitle,
      TDiagnostics,
      TEvent,
      TBlocklist,
      TSubtitle
    >,
  ) => void;
  onError?: ReactiveRefreshErrorHandler;
};

export type QueueTitleOverviewDownloadFeedbackRefreshOptions = {
  titleId: string;
  apply: (snapshot: TitleOverviewDownloadFeedbackSnapshot) => void;
  onError?: ReactiveRefreshErrorHandler;
};

export type QueueImportHistoryRefreshOptions = {
  limit?: number | null;
  apply: (records: ImportRecord[]) => void;
  onError?: ReactiveRefreshErrorHandler;
};

export type ReactiveRefreshContextValue = {
  queueCatalogTitlesRefresh: (
    options: QueueCatalogTitlesRefreshOptions,
  ) => void;
  queueCatalogTitleRefresh: (
    options: QueueCatalogTitleRefreshOptions,
  ) => void;
  queueTitleOverviewNativeRefresh: <
    TTitle = unknown,
    TDiagnostics = unknown,
    TEvent = unknown,
    TBlocklist = unknown,
    TSubtitle = unknown,
  >(
    options: QueueTitleOverviewNativeRefreshOptions<
      TTitle,
      TDiagnostics,
      TEvent,
      TBlocklist,
      TSubtitle
    >,
  ) => void;
  queueTitleOverviewDownloadFeedbackRefresh: (
    options: QueueTitleOverviewDownloadFeedbackRefreshOptions,
  ) => void;
  queueImportHistoryRefresh: (
    options: QueueImportHistoryRefreshOptions,
  ) => void;
};

export const ReactiveRefreshContext =
  createContext<ReactiveRefreshContextValue | null>(null);

export function reactiveRefreshEpoch(): number {
  return typeof performance !== "undefined" ? performance.now() : Date.now();
}

export function useReactiveRefresh(): ReactiveRefreshContextValue {
  const value = useContext(ReactiveRefreshContext);
  if (!value) {
    throw new Error(
      "useReactiveRefresh must be used within ReactiveRefreshContext.Provider",
    );
  }
  return value;
}
