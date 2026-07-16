import { useEffect, useState } from "react";

import { backendClient } from "@/lib/graphql/urql-client";
import { autoBackupSettingsQuery } from "@/lib/graphql/queries";
import { acknowledgeAutoBackupDisabledMissingKeyNoticeMutation } from "@/lib/graphql/mutations";
import { scheduleAfterFirstPaint } from "@/lib/utils/scheduling";
import { toast } from "@/components/ui/sonner";
import type { SettingsSection, Translate } from "@/components/root/types";

const AUTO_BACKUP_DISABLED_NOTICE_TOAST_ID =
  "auto-backup-disabled-missing-key-notice";

export function useAutoBackupNotice({
  canManageSystemSettings,
  serviceRestarting,
  viewingBackupsSettings,
  navigateTo,
  t,
}: {
  canManageSystemSettings: boolean;
  serviceRestarting: boolean;
  viewingBackupsSettings: boolean;
  navigateTo: (view: "settings", settingsSection: SettingsSection) => void;
  t: Translate;
}) {
  const [
    autoBackupDisabledMissingKeyNotice,
    setAutoBackupDisabledMissingKeyNotice,
  ] = useState(false);

  useEffect(() => {
    let cancelled = false;

    if (!canManageSystemSettings || serviceRestarting) {
      setAutoBackupDisabledMissingKeyNotice(false);
      return;
    }

    const cancelScheduledQuery = scheduleAfterFirstPaint(() => {
      void backendClient
        .query<{
          autoBackupSettings?: {
            autoBackupDisabledMissingKeyNotice?: boolean | null;
          } | null;
        }>(autoBackupSettingsQuery, {})
        .toPromise()
        .then(({ data, error }) => {
          if (cancelled || error) {
            return;
          }

          setAutoBackupDisabledMissingKeyNotice(
            data?.autoBackupSettings?.autoBackupDisabledMissingKeyNotice ===
              true,
          );
        });
    });

    return () => {
      cancelled = true;
      cancelScheduledQuery();
    };
  }, [canManageSystemSettings, serviceRestarting]);

  useEffect(() => {
    if (
      !autoBackupDisabledMissingKeyNotice ||
      !canManageSystemSettings ||
      viewingBackupsSettings
    ) {
      toast.dismiss(AUTO_BACKUP_DISABLED_NOTICE_TOAST_ID);
      return;
    }

    toast.warning(t("settings.autoBackupsDisabledMissingKeyToastTitle"), {
      id: AUTO_BACKUP_DISABLED_NOTICE_TOAST_ID,
      description: t("settings.autoBackupsDisabledMissingKeyToastDescription"),
      duration: Infinity,
      action: {
        label: t("settings.autoBackupsDisabledMissingKeyToastAction"),
        onClick: () => {
          navigateTo("settings", "backups");
        },
      },
    });

    return () => {
      toast.dismiss(AUTO_BACKUP_DISABLED_NOTICE_TOAST_ID);
    };
  }, [
    autoBackupDisabledMissingKeyNotice,
    canManageSystemSettings,
    navigateTo,
    t,
    viewingBackupsSettings,
  ]);

  useEffect(() => {
    if (
      !autoBackupDisabledMissingKeyNotice ||
      !canManageSystemSettings ||
      !viewingBackupsSettings
    ) {
      return;
    }

    let cancelled = false;
    void backendClient
      .mutation<{
        acknowledgeAutoBackupDisabledMissingKeyNotice?: {
          autoBackupDisabledMissingKeyNotice?: boolean | null;
        } | null;
      }>(acknowledgeAutoBackupDisabledMissingKeyNoticeMutation, {})
      .toPromise()
      .then(({ data, error }) => {
        if (cancelled || error) {
          return;
        }

        setAutoBackupDisabledMissingKeyNotice(
          data?.acknowledgeAutoBackupDisabledMissingKeyNotice
            ?.autoBackupDisabledMissingKeyNotice === true,
        );
      });

    return () => {
      cancelled = true;
    };
  }, [
    autoBackupDisabledMissingKeyNotice,
    canManageSystemSettings,
    viewingBackupsSettings,
  ]);
}
