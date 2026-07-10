export type AuditLogEvent = {
  sequence: number;
  eventId: string;
  occurredAt: string;
  actorKind: 'USER' | 'ANONYMOUS' | 'SYSTEM';
  actorUserId: string | null;
  actorDisplayName: string;
  titleId: string | null;
  facet: string | null;
  eventType: string;
  streamKind: string;
  streamId: string | null;
  payloadJson: unknown;
};
