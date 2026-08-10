export function subscribableProviderNotificationEvents(
  hostEventTypes: readonly string[],
  providerSupportedEvents: readonly string[],
): string[] {
  if (providerSupportedEvents.length === 0) {
    return [...hostEventTypes];
  }

  const providerEvents = new Set(providerSupportedEvents);
  return hostEventTypes.filter((eventType) => providerEvents.has(eventType));
}
