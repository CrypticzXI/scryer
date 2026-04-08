import { createContext, useContext } from "react";

import type { ImportRecord, TitleRecord } from "@/lib/types";
import type { TitleOverviewSnapshot } from "@/lib/title-overview-loader";

type ReactiveRefreshErrorHandler = (error: unknown) => void;

export type QueueCatalogTitlesRefreshOptions = {
  facet?: string | null;
  apply: (titles: TitleRecord[]) => void;
  onError?: ReactiveRefreshErrorHandler;
};

export type QueueCatalogTitleRefreshOptions = {
  titleId: string;
  apply: (title: TitleRecord | null) => void;
  onError?: ReactiveRefreshErrorHandler;
};

export type QueueTitleOverviewRefreshOptions<
  TTitle = unknown,
  TEvent = unknown,
  TBlocklist = unknown,
  TSubtitle = unknown,
> = {
  titleId: string;
  blocklistLimit: number;
  apply: (
    snapshot: TitleOverviewSnapshot<TTitle, TEvent, TBlocklist, TSubtitle>,
  ) => void;
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
  queueTitleOverviewRefresh: <
    TTitle = unknown,
    TEvent = unknown,
    TBlocklist = unknown,
    TSubtitle = unknown,
  >(
    options: QueueTitleOverviewRefreshOptions<
      TTitle,
      TEvent,
      TBlocklist,
      TSubtitle
    >,
  ) => void;
  queueImportHistoryRefresh: (
    options: QueueImportHistoryRefreshOptions,
  ) => void;
};

export const ReactiveRefreshContext =
  createContext<ReactiveRefreshContextValue | null>(null);

export function useReactiveRefresh(): ReactiveRefreshContextValue {
  const value = useContext(ReactiveRefreshContext);
  if (!value) {
    throw new Error(
      "useReactiveRefresh must be used within ReactiveRefreshContext.Provider",
    );
  }
  return value;
}