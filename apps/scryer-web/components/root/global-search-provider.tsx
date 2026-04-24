import { type ReactNode, useEffect } from "react";
import { useGlobalSearch } from "@/lib/hooks/use-global-search";
import type { Facet } from "@/lib/types";
import type { LocaleCode } from "@/lib/i18n";
import { SearchContext } from "@/lib/context/search-context";

type GlobalSearchProviderProps = {
  activeFacet: Facet;
  queueFacet: Facet;
  uiLanguage: LocaleCode;
  children: ReactNode;
};

export function GlobalSearchProvider({
  activeFacet,
  queueFacet,
  uiLanguage,
  children,
}: GlobalSearchProviderProps) {
  const searchState = useGlobalSearch({
    queueFacet,
    uiLanguage,
  });

  const { setQueueFacet, setTvdbCandidates } = searchState;
  useEffect(() => {
    setQueueFacet(activeFacet);
    setTvdbCandidates([]);
  }, [activeFacet, setQueueFacet, setTvdbCandidates]);

  return (
    <SearchContext.Provider value={searchState}>
      {children}
    </SearchContext.Provider>
  );
}
