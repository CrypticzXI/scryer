import { useCallback, useEffect, useMemo, useState } from "react";

import { backendClient } from "@/lib/graphql/urql-client";
import {
  smgScryerUpdateNoticeQuery,
  smgVersionCompatibilityNoticeQuery,
} from "@/lib/graphql/queries";
import { useSettingsSubscription } from "@/lib/hooks/use-settings-subscription";
import { scheduleAfterFirstPaint } from "@/lib/utils/scheduling";
import type {
  SmgScryerUpdateNotice,
  SmgVersionCompatibilityNotice,
} from "@/components/root/types";

const SMG_VERSION_COMPATIBILITY_NOTICE_KEY = "smg.version_compatibility_notice";
const SMG_SCRYER_UPDATE_NOTICE_KEY = "smg.scryer_update_notice";
const SMG_SCRYER_UPDATE_DISMISSED_KEY = "scryer.smgUpdate.dismissed";
const SMG_NOTICE_REFRESH_INTERVAL_MS = 5 * 60 * 1_000;

function buildSmgScryerUpdateDismissalValue(
  notice: SmgScryerUpdateNotice | null,
): string | null {
  if (!notice?.available) {
    return null;
  }
  const latest = notice.latestTag.trim() || notice.latestVersion.trim();
  if (!latest) {
    return null;
  }
  return `${latest}:${notice.latestVersion.trim()}`;
}

export function useSmgNotices() {
  const [smgVersionCompatibilityNotice, setSmgVersionCompatibilityNotice] =
    useState<SmgVersionCompatibilityNotice | null>(null);
  const [smgScryerUpdateNotice, setSmgScryerUpdateNotice] =
    useState<SmgScryerUpdateNotice | null>(null);
  const [dismissedSmgScryerUpdate, setDismissedSmgScryerUpdate] = useState(
    () => {
      if (typeof window === "undefined") {
        return "";
      }
      return (
        window.localStorage.getItem(SMG_SCRYER_UPDATE_DISMISSED_KEY) ?? ""
      );
    },
  );

  const refreshSmgVersionCompatibilityNotice = useCallback(async () => {
    try {
      const { data, error } = await backendClient
        .query<{
          smgVersionCompatibilityNotice?: SmgVersionCompatibilityNotice | null;
        }>(smgVersionCompatibilityNoticeQuery, {})
        .toPromise();
      if (error) {
        throw error;
      }
      setSmgVersionCompatibilityNotice(
        data?.smgVersionCompatibilityNotice ?? null,
      );
    } catch (error) {
      console.warn("Failed to refresh SMG version compatibility notice", error);
    }
  }, []);

  const refreshSmgScryerUpdateNotice = useCallback(async () => {
    try {
      const { data, error } = await backendClient
        .query<{
          smgScryerUpdateNotice?: SmgScryerUpdateNotice | null;
        }>(smgScryerUpdateNoticeQuery, {})
        .toPromise();
      if (error) {
        throw error;
      }
      setSmgScryerUpdateNotice(data?.smgScryerUpdateNotice ?? null);
    } catch (error) {
      console.warn("Failed to refresh SMG Scryer update notice", error);
    }
  }, []);

  useEffect(() => {
    return scheduleAfterFirstPaint(() => {
      void refreshSmgVersionCompatibilityNotice();
      void refreshSmgScryerUpdateNotice();
    });
  }, [refreshSmgScryerUpdateNotice, refreshSmgVersionCompatibilityNotice]);

  useSettingsSubscription(
    useCallback(
      (changedKeys) => {
        if (
          changedKeys.includes(SMG_VERSION_COMPATIBILITY_NOTICE_KEY) ||
          changedKeys.includes(SMG_SCRYER_UPDATE_NOTICE_KEY)
        ) {
          void refreshSmgVersionCompatibilityNotice();
          void refreshSmgScryerUpdateNotice();
        }
      },
      [refreshSmgScryerUpdateNotice, refreshSmgVersionCompatibilityNotice],
    ),
  );

  useEffect(() => {
    if (typeof window === "undefined" || typeof document === "undefined") {
      return;
    }

    const handleFocus = () => {
      void refreshSmgVersionCompatibilityNotice();
      void refreshSmgScryerUpdateNotice();
    };
    const handleVisibilityChange = () => {
      if (document.visibilityState === "visible") {
        void refreshSmgVersionCompatibilityNotice();
        void refreshSmgScryerUpdateNotice();
      }
    };

    window.addEventListener("focus", handleFocus);
    document.addEventListener("visibilitychange", handleVisibilityChange);
    const intervalId = window.setInterval(() => {
      void refreshSmgVersionCompatibilityNotice();
      void refreshSmgScryerUpdateNotice();
    }, SMG_NOTICE_REFRESH_INTERVAL_MS);
    return () => {
      window.removeEventListener("focus", handleFocus);
      document.removeEventListener("visibilitychange", handleVisibilityChange);
      window.clearInterval(intervalId);
    };
  }, [refreshSmgScryerUpdateNotice, refreshSmgVersionCompatibilityNotice]);

  const smgScryerUpdateDismissalValue = useMemo(
    () => buildSmgScryerUpdateDismissalValue(smgScryerUpdateNotice),
    [smgScryerUpdateNotice],
  );
  const showSmgScryerUpdateReminder =
    !smgVersionCompatibilityNotice &&
    Boolean(smgScryerUpdateNotice?.available) &&
    Boolean(smgScryerUpdateDismissalValue) &&
    dismissedSmgScryerUpdate !== smgScryerUpdateDismissalValue;

  const dismissSmgScryerUpdateReminder = useCallback(() => {
    if (!smgScryerUpdateDismissalValue) {
      return;
    }
    setDismissedSmgScryerUpdate(smgScryerUpdateDismissalValue);
    if (typeof window !== "undefined") {
      window.localStorage.setItem(
        SMG_SCRYER_UPDATE_DISMISSED_KEY,
        smgScryerUpdateDismissalValue,
      );
    }
  }, [smgScryerUpdateDismissalValue]);

  return {
    smgVersionCompatibilityNotice,
    smgScryerUpdateNotice,
    showSmgScryerUpdateReminder,
    dismissSmgScryerUpdateReminder,
  };
}
