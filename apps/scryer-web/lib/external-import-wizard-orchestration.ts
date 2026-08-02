import type { ExternalImportMonitorWarmupStatus } from "@/lib/types/external-import";

export function continueExternalImportFromConnect(
  loadPreview: () => Promise<void>,
  navigate: () => void,
): void {
  void loadPreview();
  navigate();
}

export function isProwlarrDiscoveryReady(
  hasConnectedProwlarr: boolean,
  sessionId: string | null,
  status: ExternalImportMonitorWarmupStatus | null,
): boolean {
  return (
    !hasConnectedProwlarr ||
    (Boolean(sessionId) && status === "COMPLETED")
  );
}

export function canRetryProwlarrDiscovery(
  status: ExternalImportMonitorWarmupStatus | null,
): boolean {
  return status === "FAILED" || status === "CANCELED";
}
