export type UserRecord = {
  id: string;
  username: string;
  appPermissions: string[];
  libraryPermissions: {
    libraryId: string;
    permissions: string[];
  }[];
};
