import { createContext, useContext } from "react";

export type GlobalStatusOptions = {
  toastId?: string;
};

export type SetGlobalStatus = (status: string, options?: GlobalStatusOptions) => void;

export const GlobalStatusContext = createContext<SetGlobalStatus | null>(null);

export function useGlobalStatus(): SetGlobalStatus {
  const fn = useContext(GlobalStatusContext);
  if (!fn) throw new Error("useGlobalStatus must be used within GlobalStatusContext.Provider");
  return fn;
}
