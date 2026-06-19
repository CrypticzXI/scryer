// Authored preview for the Scryer <Dialog> — rendered open (defaultOpen) so the modal shows.
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription, DialogFooter, Button } from "scryer-web";

export function Default() {
  return (
    <Dialog defaultOpen>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Delete movie?</DialogTitle>
          <DialogDescription>
            This removes “Dune: Part Two (2024)” from your library. Files already on disk are kept.
          </DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button variant="ghost">Cancel</Button>
          <Button variant="destructive">Delete</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
