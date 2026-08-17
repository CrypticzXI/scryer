import type { InteractiveSearchIndexerProgress } from "@/lib/graphql/release-search";

type InteractiveSearchPresentationInput = {
  hasSnapshot: boolean;
  loading: boolean;
  resultCount: number;
  indexers: InteractiveSearchIndexerProgress[];
};

export type InteractiveSearchPresentation = {
  showInitialLoader: boolean;
  showResults: boolean;
  showProgress: boolean;
  showFinalSummary: boolean;
  completedIndexerCount: number;
  totalIndexerCount: number;
  failedIndexerNames: string[];
};

export function deriveInteractiveSearchPresentation({
  hasSnapshot,
  loading,
  resultCount,
  indexers,
}: InteractiveSearchPresentationInput): InteractiveSearchPresentation {
  const completedIndexerCount = indexers.filter(
    (indexer) =>
      indexer.status === "COMPLETED" ||
      indexer.status === "FAILED" ||
      indexer.status === "SKIPPED",
  ).length;

  return {
    showInitialLoader: loading && resultCount === 0,
    showResults: resultCount > 0,
    showProgress: loading,
    showFinalSummary: hasSnapshot && !loading,
    completedIndexerCount,
    totalIndexerCount: indexers.length,
    failedIndexerNames: indexers
      .filter((indexer) => indexer.status === "FAILED")
      .map((indexer) => indexer.name),
  };
}
