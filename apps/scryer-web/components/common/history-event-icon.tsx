import { getTitleHistoryEventMeta } from "./title-history-event-meta";

export function HistoryEventIcon({
  eventType,
  size = 16,
}: {
  eventType: string;
  size?: number;
}) {
  const config = getTitleHistoryEventMeta(eventType);
  const Icon = config.icon;
  return (
    <Icon
      style={{ width: size, height: size }}
      className={`shrink-0 ${config.iconClassName}`}
      aria-label={config.label}
    />
  );
}

export function getEventTypeLabel(eventType: string): string {
  return getTitleHistoryEventMeta(eventType).label;
}

export function getEventTypeBadgeClass(eventType: string): string {
  return getTitleHistoryEventMeta(eventType).badgeClassName;
}
