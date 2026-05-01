import type { ConfigFieldDef } from "./indexers";
import type { TitleRecord } from "./titles";

export type NotificationChannel = {
  id: string;
  name: string;
  channelType: string;
  configJson: string | null;
  isEnabled: boolean;
  createdAt: string;
  updatedAt: string;
};

export type NotificationChannelDraft = {
  name: string;
  channelType: string;
  isEnabled: boolean;
  configValues: Record<string, string>;
};

export type NotificationSubscription = {
  id: string;
  channelId: string;
  eventType: string;
  scope: string;
  scopeId: string | null;
  isEnabled: boolean;
  createdAt: string;
  updatedAt: string;
};

export type NotificationSubscriptionDraft = {
  channelId: string;
  eventTypes: string[];
  scope: string;
  facetScopeIds: string[];
  titleScopeId: string;
  titleScopeTitle: TitleRecord | null;
  isEnabled: boolean;
};

export type NotificationSubscriptionRow = {
  id: string;
  channelId: string;
  eventTypes: string[];
  scope: string;
  scopeId: string | null;
  isEnabled: boolean;
  subscriptionIds: string[];
};

export type NotificationProviderType = {
  providerType: string;
  name: string;
  defaultBaseUrl: string | null;
  configFields: ConfigFieldDef[];
};
