// Authored preview for the Scryer <Toaster> (sonner). Toasts are imperative, so we push a
// few on mount (long duration) so the card shows real toast styling rather than an empty
// container. See NOTES.md — if capture timing leaves it blank, this falls back to a floor card.
import React from "react";
import { Toaster, toast } from "scryer-web";

export function Notifications() {
  React.useEffect(() => {
    toast.success("Dune: Part Two imported", {
      description: "2160p · BluRay · 18.4 GB",
      duration: 1000000,
    });
    toast.error("Indexer “NZBgeek” unreachable", { duration: 1000000 });
    toast("Refreshing library…", { duration: 1000000 });
  }, []);
  return <Toaster position="top-center" expand />;
}
