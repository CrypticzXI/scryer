export type AuditLogEvent = {
  sequence: number;
  eventId: string;
  occurredAt: string;
  actorKind: string;
  actorUserId: string | null;
  actorDisplayName: string;
  titleId: string | null;
  facet: string | null;
  eventType: string;
  streamKind: string;
  streamId: string | null;
  payloadJson: unknown;
};
