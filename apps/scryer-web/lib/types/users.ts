export type UserAccountKind = "local" | "external_auto_provisioned";

export type UserRecord = {
  id: string;
  username: string;
  hasPassword: boolean;
  hasMfa: boolean;
  hasPasskey: boolean;
  accountKind: UserAccountKind;
  appPermissions: string[];
  libraryPermissions: {
    libraryId: string;
    permissions: string[];
  }[];
};
