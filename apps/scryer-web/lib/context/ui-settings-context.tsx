import * as React from "react";
import { useClient } from "urql";
import { myUiSettingsQuery } from "@/lib/graphql/queries";
import type { UiSettings } from "@/lib/types/settings";

export const DEFAULT_UI_SETTINGS: UiSettings = {
  theme: "dark",
  dateTimeFormat: "locale",
  highlightColor: null,
  secondaryColor: null,
  highContrastMode: false,
  reduceMotion: false,
  density: "comfortable",
  sidebarMode: "expanded",
  defaultLandingView: "movies",
  tableColumns: [],
};

type UiSettingsContextValue = {
  uiSettings: UiSettings;
  uiSettingsLoading: boolean;
  setUiSettings: (settings: UiSettings) => void;
  refreshUiSettings: () => Promise<void>;
};

const UiSettingsContext = React.createContext<UiSettingsContextValue | null>(null);

function normalizeUiSettings(settings: Partial<UiSettings> | null | undefined): UiSettings {
  return {
    ...DEFAULT_UI_SETTINGS,
    ...settings,
    tableColumns: settings?.tableColumns ?? [],
  };
}

export function uiSettingsInputFromSettings(settings: UiSettings): UiSettings {
  return {
    ...settings,
    tableColumns: settings.tableColumns.map((column) => ({ ...column })),
  };
}

export function UiSettingsProvider({ children }: { children: React.ReactNode }) {
  const client = useClient();
  const [uiSettings, setUiSettings] = React.useState<UiSettings>(DEFAULT_UI_SETTINGS);
  const [uiSettingsLoading, setUiSettingsLoading] = React.useState(true);

  const refreshUiSettings = React.useCallback(async () => {
    setUiSettingsLoading(true);
    try {
      const { data, error } = await client
        .query<{ myUiSettings: UiSettings }>(myUiSettingsQuery, {})
        .toPromise();
      if (error) throw error;
      setUiSettings(normalizeUiSettings(data?.myUiSettings));
    } catch {
      setUiSettings(DEFAULT_UI_SETTINGS);
    } finally {
      setUiSettingsLoading(false);
    }
  }, [client]);

  React.useEffect(() => {
    let cancelled = false;
    setUiSettingsLoading(true);
    (async () => {
      try {
        const { data, error } = await client
          .query<{ myUiSettings: UiSettings }>(myUiSettingsQuery, {})
          .toPromise();
        if (error) throw error;
        if (!cancelled) {
          setUiSettings(normalizeUiSettings(data?.myUiSettings));
        }
      } catch {
        if (!cancelled) {
          setUiSettings(DEFAULT_UI_SETTINGS);
        }
      } finally {
        if (!cancelled) {
          setUiSettingsLoading(false);
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [client]);

  const value = React.useMemo<UiSettingsContextValue>(
    () => ({
      uiSettings,
      uiSettingsLoading,
      setUiSettings,
      refreshUiSettings,
    }),
    [refreshUiSettings, uiSettings, uiSettingsLoading],
  );

  return (
    <UiSettingsContext.Provider value={value}>
      {children}
    </UiSettingsContext.Provider>
  );
}

export function useUiSettings() {
  const value = React.useContext(UiSettingsContext);
  if (!value) {
    throw new Error("useUiSettings must be used within UiSettingsProvider");
  }
  return value;
}

export function useUiDateTimeFormat() {
  return useUiSettings().uiSettings.dateTimeFormat;
}
