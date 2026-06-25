import * as React from "react";
import { useClient } from "urql";
import { myUiSettingsQuery } from "@/lib/graphql/queries";
import { AUTH_SESSION_CHANGED_EVENT } from "@/lib/hooks/use-auth";
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
  uiSettingsLoaded: boolean;
  uiSettingsLoadError: string | null;
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

function uiSettingsErrorMessage(error: unknown): string {
  return error instanceof Error ? error.message : "Failed to load UI settings";
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
  const [uiSettingsLoaded, setUiSettingsLoaded] = React.useState(false);
  const [uiSettingsLoadError, setUiSettingsLoadError] = React.useState<string | null>(null);
  const requestSequenceRef = React.useRef(0);

  const loadUiSettings = React.useCallback(async (options?: { resetToFallback?: boolean }) => {
    const requestId = requestSequenceRef.current + 1;
    requestSequenceRef.current = requestId;

    if (options?.resetToFallback) {
      setUiSettings(DEFAULT_UI_SETTINGS);
    }
    setUiSettingsLoading(true);
    setUiSettingsLoaded(false);
    setUiSettingsLoadError(null);
    try {
      const { data, error } = await client
        .query<{ myUiSettings: UiSettings }>(myUiSettingsQuery, {})
        .toPromise();
      if (error) throw error;
      if (requestSequenceRef.current !== requestId) return;
      setUiSettings(normalizeUiSettings(data?.myUiSettings));
      setUiSettingsLoaded(true);
    } catch (error) {
      if (requestSequenceRef.current !== requestId) return;
      setUiSettings(DEFAULT_UI_SETTINGS);
      setUiSettingsLoaded(false);
      setUiSettingsLoadError(uiSettingsErrorMessage(error));
    } finally {
      if (requestSequenceRef.current === requestId) {
        setUiSettingsLoading(false);
      }
    }
  }, [client]);

  const refreshUiSettings = React.useCallback(
    () => loadUiSettings(),
    [loadUiSettings],
  );

  React.useEffect(() => {
    if (typeof window === "undefined") {
      void loadUiSettings({ resetToFallback: true });
      return undefined;
    }

    const handleAuthSessionChanged = () => {
      void loadUiSettings({ resetToFallback: true });
    };

    window.addEventListener(AUTH_SESSION_CHANGED_EVENT, handleAuthSessionChanged);
    void loadUiSettings({ resetToFallback: true });
    return () => {
      requestSequenceRef.current += 1;
      window.removeEventListener(AUTH_SESSION_CHANGED_EVENT, handleAuthSessionChanged);
    };
  }, [loadUiSettings]);

  const value = React.useMemo<UiSettingsContextValue>(
    () => ({
      uiSettings,
      uiSettingsLoading,
      uiSettingsLoaded,
      uiSettingsLoadError,
      setUiSettings,
      refreshUiSettings,
    }),
    [
      refreshUiSettings,
      uiSettings,
      uiSettingsLoaded,
      uiSettingsLoadError,
      uiSettingsLoading,
    ],
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
