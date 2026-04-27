import type { Release } from "@/lib/types";

export type EpisodePanelTab = "details" | "search" | "blocklist";

export interface EpisodePanelState {
  searchResultsByEpisode: Record<string, Release[]>;
  searchLoadingByEpisode: Record<string, boolean>;
  autoSearchLoadingByEpisode: Record<string, boolean>;
}

export type EpisodePanelAction =
  | { type: "SET_SEARCH_RESULTS"; episodeId: string; results: Release[] }
  | { type: "SET_SEARCH_LOADING"; episodeId: string; loading: boolean }
  | { type: "SET_AUTO_SEARCH_LOADING"; episodeId: string; loading: boolean };

export const initialEpisodePanelState: EpisodePanelState = {
  searchResultsByEpisode: {},
  searchLoadingByEpisode: {},
  autoSearchLoadingByEpisode: {},
};

export function episodePanelReducer(
  state: EpisodePanelState,
  action: EpisodePanelAction,
): EpisodePanelState {
  switch (action.type) {
    case "SET_SEARCH_RESULTS":
      return {
        ...state,
        searchResultsByEpisode: {
          ...state.searchResultsByEpisode,
          [action.episodeId]: action.results,
        },
      };

    case "SET_SEARCH_LOADING": {
      if (action.loading) {
        return {
          ...state,
          searchLoadingByEpisode: {
            ...state.searchLoadingByEpisode,
            [action.episodeId]: true,
          },
        };
      }
      const { [action.episodeId]: _removed, ...rest } = state.searchLoadingByEpisode;
      return { ...state, searchLoadingByEpisode: rest };
    }

    case "SET_AUTO_SEARCH_LOADING": {
      if (action.loading) {
        return {
          ...state,
          autoSearchLoadingByEpisode: {
            ...state.autoSearchLoadingByEpisode,
            [action.episodeId]: true,
          },
        };
      }
      const { [action.episodeId]: _removed, ...rest } = state.autoSearchLoadingByEpisode;
      return { ...state, autoSearchLoadingByEpisode: rest };
    }

    default:
      return state;
  }
}
