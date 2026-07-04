export type CanonicalMediaTag = {
  key: string;
  category: string;
  name: string;
  confidence?: number | null;
  sources: string[];
  sourceTagKeys: string[];
  isAdult: boolean;
  isSpoiler: boolean;
};
