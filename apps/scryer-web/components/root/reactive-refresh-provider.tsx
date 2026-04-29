import { type ReactNode, useCallback, useEffect, useMemo, useRef } from "react";
import { CombinedError, useClient } from "urql";

import {
  buildReactiveRefreshQuery,
  type ReactiveRefreshQueryActionInput,
  type ReactiveRefreshQueryActionPlan,
} from "@/lib/graphql/queries";
import { extractDownloadFeedbackWarning } from "@/lib/graphql/download-feedback-timeout";
import {
  ReactiveRefreshContext,
  type QueueCatalogTitleRefreshOptions,
  type QueueCatalogTitlesRefreshOptions,
  type QueueImportHistoryRefreshOptions,
  type QueueTitleOverviewDownloadFeedbackRefreshOptions,
  type QueueTitleOverviewNativeRefreshOptions,
  type ReactiveRefreshContextValue,
} from "@/lib/context/reactive-refresh-context";
import type { ImportRecord, TitleRecord } from "@/lib/types";
import type {
  TitleOverviewDownloadFeedbackSnapshot,
  TitleOverviewNativeSnapshot,
} from "@/lib/title-overview-loader";

type CatalogTitlesAction = {
  key: string;
  kind: "catalogTitles";
} & QueueCatalogTitlesRefreshOptions;

type CatalogTitleAction = {
  key: string;
  kind: "catalogTitle";
} & QueueCatalogTitleRefreshOptions;

type TitleOverviewNativeAction = {
  key: string;
  kind: "titleOverviewNative";
  titleId: string;
  blocklistLimit: number;
  apply: (
    snapshot: TitleOverviewNativeSnapshot<
      unknown,
      unknown,
      unknown,
      unknown,
      unknown
    >,
  ) => void;
  onError?: QueueTitleOverviewNativeRefreshOptions["onError"];
};

type TitleOverviewDownloadFeedbackAction = {
  key: string;
  kind: "titleOverviewDownloadFeedback";
  titleId: string;
  apply: (snapshot: TitleOverviewDownloadFeedbackSnapshot) => void;
  onError?: QueueTitleOverviewDownloadFeedbackRefreshOptions["onError"];
};

type ImportHistoryAction = {
  key: string;
  kind: "importHistory";
} & QueueImportHistoryRefreshOptions;

type ReactiveRefreshAction =
  | CatalogTitlesAction
  | CatalogTitleAction
  | TitleOverviewNativeAction
  | TitleOverviewDownloadFeedbackAction
  | ImportHistoryAction;

type ReactiveRefreshBatchGroup = "default" | "downloadFeedback";

const REACTIVE_REFRESH_DEBOUNCE_MS = 300;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function actionInputFromPendingAction(
  action: ReactiveRefreshAction,
): ReactiveRefreshQueryActionInput {
  switch (action.kind) {
    case "catalogTitles":
      return {
        key: action.key,
        kind: action.kind,
        facet: action.facet,
      };
    case "catalogTitle":
      return {
        key: action.key,
        kind: action.kind,
        titleId: action.titleId,
      };
    case "titleOverviewNative":
      return {
        key: action.key,
        kind: action.kind,
        titleId: action.titleId,
        blocklistLimit: action.blocklistLimit,
      };
    case "titleOverviewDownloadFeedback":
      return {
        key: action.key,
        kind: action.kind,
        titleId: action.titleId,
      };
    case "importHistory":
      return {
        key: action.key,
        kind: action.kind,
        limit: action.limit,
      };
    default: {
      const exhaustiveCheck: never = action;
      throw new Error(`unsupported reactive refresh action: ${exhaustiveCheck}`);
    }
  }
}

function applyReactiveRefreshActionResult(
  action: ReactiveRefreshAction,
  actionPlan: ReactiveRefreshQueryActionPlan,
  payload: Record<string, unknown>,
  downloadFeedbackWarning: string | null = null,
) {
  switch (action.kind) {
    case "catalogTitles": {
      const typedActionPlan = actionPlan as Extract<
        ReactiveRefreshQueryActionPlan,
        { kind: "catalogTitles" }
      >;
      action.apply((payload[typedActionPlan.titlesAlias] ?? []) as TitleRecord[]);
      return;
    }
    case "catalogTitle": {
      const typedActionPlan = actionPlan as Extract<
        ReactiveRefreshQueryActionPlan,
        { kind: "catalogTitle" }
      >;
      action.apply(
        (payload[typedActionPlan.titleAlias] ?? null) as TitleRecord | null,
      );
      return;
    }
    case "titleOverviewNative": {
      const typedActionPlan = actionPlan as Extract<
        ReactiveRefreshQueryActionPlan,
        { kind: "titleOverviewNative" }
      >;
      action.apply({
        title: payload[typedActionPlan.titleAlias] ?? null,
        acquisitionDiagnostics:
          payload[typedActionPlan.titleAcquisitionDiagnosticsAlias] ?? null,
        titleEvents: (payload[typedActionPlan.titleEventsAlias] ?? []) as TitleOverviewNativeSnapshot<
          unknown,
          unknown,
          unknown,
          unknown,
          unknown
        >["titleEvents"],
        titleReleaseBlocklist: (payload[typedActionPlan.titleReleaseBlocklistAlias] ?? []) as TitleOverviewNativeSnapshot<
          unknown,
          unknown,
          unknown,
          unknown,
          unknown
        >["titleReleaseBlocklist"],
        externalSubtitles: (payload[typedActionPlan.externalSubtitlesAlias] ?? []) as TitleOverviewNativeSnapshot<
          unknown,
          unknown,
          unknown,
          unknown,
          unknown
        >["externalSubtitles"],
        hasDownloadClients: (payload[typedActionPlan.setupStatusAlias] as { hasDownloadClients?: boolean } | null | undefined)?.hasDownloadClients === true,
      });
      return;
    }
    case "titleOverviewDownloadFeedback": {
      const typedActionPlan = actionPlan as Extract<
        ReactiveRefreshQueryActionPlan,
        { kind: "titleOverviewDownloadFeedback" }
      >;
      action.apply({
        downloadQueueItems:
          (payload[typedActionPlan.downloadQueueItemsAlias] ?? []) as TitleOverviewDownloadFeedbackSnapshot["downloadQueueItems"],
        completedDownloadQueueItems:
          (payload[typedActionPlan.completedDownloadQueueItemsAlias] ?? []) as TitleOverviewDownloadFeedbackSnapshot["completedDownloadQueueItems"],
        downloadFeedbackWarning,
      });
      return;
    }
    case "importHistory": {
      const typedActionPlan = actionPlan as Extract<
        ReactiveRefreshQueryActionPlan,
        { kind: "importHistory" }
      >;
      action.apply(
        (payload[typedActionPlan.importHistoryAlias] ?? []) as ImportRecord[],
      );
      return;
    }
    default: {
      const exhaustiveCheck: never = action;
      throw new Error(`unsupported reactive refresh action: ${exhaustiveCheck}`);
    }
  }
}

function reactiveRefreshActionAliases(
  actionPlan: ReactiveRefreshQueryActionPlan,
): string[] {
  switch (actionPlan.kind) {
    case "catalogTitles":
      return [actionPlan.titlesAlias];
    case "catalogTitle":
      return [actionPlan.titleAlias];
    case "titleOverviewNative":
      return [
        actionPlan.titleAlias,
        actionPlan.titleAcquisitionDiagnosticsAlias,
        actionPlan.titleEventsAlias,
        actionPlan.titleReleaseBlocklistAlias,
        actionPlan.externalSubtitlesAlias,
        actionPlan.setupStatusAlias,
      ];
    case "titleOverviewDownloadFeedback":
      return [
        actionPlan.downloadQueueItemsAlias,
        actionPlan.completedDownloadQueueItemsAlias,
      ];
    case "importHistory":
      return [actionPlan.importHistoryAlias];
    default: {
      const exhaustiveCheck: never = actionPlan;
      throw new Error(`unsupported reactive refresh action: ${exhaustiveCheck}`);
    }
  }
}

function graphQlErrorAlias(value: unknown): string | null {
  if (!isRecord(value) || !Array.isArray(value.path) || value.path.length === 0) {
    return null;
  }

  return typeof value.path[0] === "string" ? value.path[0] : null;
}

function routeActionScopedErrors(
  actionPlans: ReactiveRefreshQueryActionPlan[],
  error: CombinedError,
) {
  const actionKeyByAlias = new Map<string, string>();
  actionPlans.forEach((actionPlan) => {
    reactiveRefreshActionAliases(actionPlan).forEach((alias) => {
      actionKeyByAlias.set(alias, actionPlan.key);
    });
  });

  const graphQlErrorsByKey = new Map<
    string,
    Array<CombinedError["graphQLErrors"][number]>
  >();
  let hasUnscopedGraphQlErrors = false;

  error.graphQLErrors.forEach((graphQlError) => {
    const alias = graphQlErrorAlias(graphQlError);
    if (!alias) {
      hasUnscopedGraphQlErrors = true;
      return;
    }

    const actionKey = actionKeyByAlias.get(alias);
    if (!actionKey) {
      hasUnscopedGraphQlErrors = true;
      return;
    }

    const existingErrors = graphQlErrorsByKey.get(actionKey) ?? [];
    existingErrors.push(graphQlError);
    graphQlErrorsByKey.set(actionKey, existingErrors);
  });

  const actionErrorsByKey = new Map<string, CombinedError>();
  graphQlErrorsByKey.forEach((graphQlErrors, actionKey) => {
    actionErrorsByKey.set(
      actionKey,
      new CombinedError({
        graphQLErrors: graphQlErrors,
        response: error.response,
      }),
    );
  });

  return {
    actionErrorsByKey,
    failedActionKeys: new Set(actionErrorsByKey.keys()),
    hasUnscopedGraphQlErrors,
  };
}

function reactiveRefreshBatchGroup(
  action: ReactiveRefreshAction,
): ReactiveRefreshBatchGroup {
  return action.kind === "titleOverviewDownloadFeedback"
    ? "downloadFeedback"
    : "default";
}

export function ReactiveRefreshProvider({
  children,
}: {
  children: ReactNode;
}) {
  const client = useClient();
  const pendingActionsRef = useRef<Map<string, ReactiveRefreshAction>>(new Map());
  const flushTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const flushInFlightRef = useRef(false);
  const isMountedRef = useRef(true);

  const flushPendingActionGroup = useCallback(async (queuedActions: ReactiveRefreshAction[]) => {
    if (queuedActions.length === 0) {
      return;
    }

    try {
      const queryPlan = buildReactiveRefreshQuery(
        queuedActions.map(actionInputFromPendingAction),
      );
      const { data, error } = await client
        .query(queryPlan.query, queryPlan.variables, {
          requestPolicy: "network-only",
        })
        .toPromise();
      const payload = isRecord(data) ? data : {};
      if (!isMountedRef.current) {
        return;
      }

      const actionsByKey = new Map(queuedActions.map((action) => [action.key, action]));
      const actionPlansByKey = new Map(
        queryPlan.actionPlans.map((actionPlan) => [actionPlan.key, actionPlan]),
      );
      const failedActionKeys = new Set<string>();
      const actionErrorsByKey = new Map<string, CombinedError>();
      const titleOverviewDownloadFeedbackWarningsByKey = new Map<string, string>();

      if (error) {
        if (error.networkError || !isRecord(data)) {
          throw error;
        }

        const routedErrors = routeActionScopedErrors(queryPlan.actionPlans, error);
        if (routedErrors.hasUnscopedGraphQlErrors) {
          throw error;
        }

        routedErrors.actionErrorsByKey.forEach((actionError, actionKey) => {
          const actionPlan = actionPlansByKey.get(actionKey);
          if (actionPlan?.kind === "titleOverviewDownloadFeedback") {
            const warning = extractDownloadFeedbackWarning(actionError.graphQLErrors, [
              actionPlan.downloadQueueItemsAlias,
              actionPlan.completedDownloadQueueItemsAlias,
            ]);
            if (warning) {
              titleOverviewDownloadFeedbackWarningsByKey.set(actionKey, warning);
              return;
            }
          }

          failedActionKeys.add(actionKey);
          actionErrorsByKey.set(actionKey, actionError);
        });
      }

      queryPlan.actionPlans.forEach((actionPlan) => {
        if (failedActionKeys.has(actionPlan.key)) {
          return;
        }

        const action = actionsByKey.get(actionPlan.key);
        if (!action) {
          return;
        }
        applyReactiveRefreshActionResult(
          action,
          actionPlan,
          payload,
          titleOverviewDownloadFeedbackWarningsByKey.get(actionPlan.key) ?? null,
        );
      });

      actionErrorsByKey.forEach((actionError, actionKey) => {
        actionsByKey.get(actionKey)?.onError?.(actionError);
      });
    } catch (error) {
      queuedActions.forEach((action) => {
        action.onError?.(error);
      });
    }
  }, [client]);

  const flushPendingActions = useCallback(async () => {
    if (flushInFlightRef.current) {
      return;
    }

    const queuedActions = Array.from(pendingActionsRef.current.values());
    pendingActionsRef.current.clear();
    if (queuedActions.length === 0) {
      return;
    }

    flushInFlightRef.current = true;

    try {
      const groupedActions = new Map<ReactiveRefreshBatchGroup, ReactiveRefreshAction[]>();
      queuedActions.forEach((action) => {
        const batchGroup = reactiveRefreshBatchGroup(action);
        const existingGroup = groupedActions.get(batchGroup);
        if (existingGroup) {
          existingGroup.push(action);
          return;
        }
        groupedActions.set(batchGroup, [action]);
      });

      await Promise.all(
        Array.from(groupedActions.values()).map((actionGroup) =>
          flushPendingActionGroup(actionGroup)
        ),
      );
    } finally {
      flushInFlightRef.current = false;
      if (pendingActionsRef.current.size > 0 && !flushTimerRef.current) {
        flushTimerRef.current = setTimeout(() => {
          flushTimerRef.current = null;
          void flushPendingActions();
        }, REACTIVE_REFRESH_DEBOUNCE_MS);
      }
    }
  }, [flushPendingActionGroup]);

  const queuePendingAction = useCallback(
    (action: ReactiveRefreshAction) => {
      pendingActionsRef.current.set(action.key, action);
      if (flushTimerRef.current || flushInFlightRef.current) {
        return;
      }

      flushTimerRef.current = setTimeout(() => {
        flushTimerRef.current = null;
        void flushPendingActions();
      }, REACTIVE_REFRESH_DEBOUNCE_MS);
    },
    [flushPendingActions],
  );

  useEffect(() => {
    // React StrictMode runs effect cleanup before the final mounted pass in dev,
    // so reset the mount flag here before any queued refresh responses apply.
    isMountedRef.current = true;
    const pendingActions = pendingActionsRef.current;

    return () => {
      isMountedRef.current = false;
      if (flushTimerRef.current) {
        clearTimeout(flushTimerRef.current);
        flushTimerRef.current = null;
      }
      pendingActions.clear();
    };
  }, []);

  const value = useMemo<ReactiveRefreshContextValue>(
    () => ({
      queueCatalogTitlesRefresh(options: QueueCatalogTitlesRefreshOptions) {
        queuePendingAction({
          ...options,
          key: `catalogTitles:${options.facet ?? "all"}`,
          kind: "catalogTitles",
        });
      },
      queueCatalogTitleRefresh(options: QueueCatalogTitleRefreshOptions) {
        queuePendingAction({
          ...options,
          key: `catalogTitle:${options.titleId}`,
          kind: "catalogTitle",
        });
      },
        queueTitleOverviewNativeRefresh<
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
      ) {
        queuePendingAction({
          ...options,
            key: `titleOverviewNative:${options.titleId}:${options.blocklistLimit}`,
            kind: "titleOverviewNative",
          } as TitleOverviewNativeAction);
        },
        queueTitleOverviewDownloadFeedbackRefresh(
          options: QueueTitleOverviewDownloadFeedbackRefreshOptions,
        ) {
          queuePendingAction({
            ...options,
            key: `titleOverviewDownloadFeedback:${options.titleId}`,
            kind: "titleOverviewDownloadFeedback",
          } as TitleOverviewDownloadFeedbackAction);
      },
      queueImportHistoryRefresh(options: QueueImportHistoryRefreshOptions) {
        queuePendingAction({
          ...options,
          key: `importHistory:${options.limit ?? "default"}`,
          kind: "importHistory",
        });
      },
    }),
    [queuePendingAction],
  );

  return (
    <ReactiveRefreshContext.Provider value={value}>
      {children}
    </ReactiveRefreshContext.Provider>
  );
}
