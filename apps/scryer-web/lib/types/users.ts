export type UserAccountKind = "LOCAL" | "EXTERNAL_AUTO_PROVISIONED";

export type UserRecord = {
  id: string;
  username: string;
  loginEnabled: boolean;
  isDefaultAdmin: boolean;
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
