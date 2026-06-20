import { createElement, useCallback, useRef } from "react";
import { toast } from "sonner";

import type { GlobalStatusOptions, SetGlobalStatus } from "@/lib/context/global-status-context";
import { normalizeGraphQlErrorMessage } from "@/lib/graphql/error-message";
import { classifyStatusToastLevel } from "@/lib/utils/status-toast";

type UseGlobalStatusToastOptions = {
  dedupeMs?: number;
};

const DEFAULT_DEDUPE_MS = 1200;

export function useGlobalStatusToast(setGlobalStatus: SetGlobalStatus, {
  dedupeMs = DEFAULT_DEDUPE_MS,
}: UseGlobalStatusToastOptions = {}) {
  const lastToastRef = useRef({
    key: "",
    at: 0,
  });

  return useCallback((rawStatus: string, options?: GlobalStatusOptions) => {
    setGlobalStatus(rawStatus);

    const toastLevel = classifyStatusToastLevel(rawStatus);
    if (!toastLevel) {
      return;
    }

    const displayStatus = normalizeGraphQlErrorMessage(rawStatus) || rawStatus.trim();

    const now = Date.now();
    const key = `${toastLevel}:${displayStatus.trim()}`;
    if (lastToastRef.current.key === key && now - lastToastRef.current.at < dedupeMs) {
      return;
    }

    const content = options?.toastId
      ? createElement("span", { id: options.toastId }, displayStatus)
      : displayStatus;
    const toastOptions = options?.toastId ? { id: options.toastId } : undefined;

    if (toastLevel === "success") {
      toast.success(content, toastOptions);
    } else if (toastLevel === "error") {
      toast.error(content, toastOptions);
    } else {
      toast.warning(content, toastOptions);
    }

    lastToastRef.current = { key, at: now };
  }, [dedupeMs, setGlobalStatus]);
}
