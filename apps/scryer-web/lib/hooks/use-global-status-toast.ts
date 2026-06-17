import { useCallback, useRef } from "react";
import { toast } from "sonner";

import { normalizeGraphQlErrorMessage } from "@/lib/graphql/error-message";
import { classifyStatusToastLevel } from "@/lib/utils/status-toast";

type SetGlobalStatus = (status: string) => void;

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

  return useCallback((rawStatus: string) => {
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

    if (toastLevel === "success") {
      toast.success(displayStatus);
    } else if (toastLevel === "error") {
      toast.error(displayStatus);
    } else {
      toast.warning(displayStatus);
    }

    lastToastRef.current = { key, at: now };
  }, [dedupeMs, setGlobalStatus]);
}
