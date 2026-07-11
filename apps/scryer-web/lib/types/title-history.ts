export type TitleHistoryEvent = {
  id: string;
  titleId: string;
  titleName: string | null;
  facet: string | null;
  episodeId: string | null;
  episodeIds: string[];
  collectionId: string | null;
  eventType: string;
  actorKind: 'USER' | 'ANONYMOUS' | 'SYSTEM' | null;
  actorUserId: string | null;
  actorDisplayName: string | null;
  sourceTitle: string | null;
  displayTitle: string | null;
  sourceSystem: string | null;
  sourceRef: string | null;
  sourceHint: string | null;
  quality: string | null;
  downloadId: string | null;
  clientId: string | null;
  clientName: string | null;
  importId: string | null;
  skipReason: string | null;
  retryRequiresPassword: boolean;
  failureReason: string | null;
  blocklistReason: string | null;
  sourcePath: string | null;
  destPath: string | null;
  dataJson: unknown;
  occurredAt: string;
  createdAt: string;
};

export type TitleHistoryPage = {
  items: TitleHistoryEvent[];
  totalCount: number;
  hasMore: boolean;
};
