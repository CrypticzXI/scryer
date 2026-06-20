// Authored preview for the Scryer <Sheet> — rendered open (right side) so the panel shows.
import { Sheet, SheetContent, SheetHeader, SheetTitle, SheetDescription, SheetFooter, Button, Label, Input } from "scryer-web";

export function Right() {
  return (
    <Sheet defaultOpen>
      <SheetContent side="right">
        <SheetHeader>
          <SheetTitle>Edit indexer</SheetTitle>
          <SheetDescription>Configure how Scryer queries this indexer.</SheetDescription>
        </SheetHeader>
        <div className="flex flex-col gap-4 px-4">
          <div className="flex flex-col gap-2">
            <Label htmlFor="s-name">Name</Label>
            <Input id="s-name" defaultValue="NZBgeek" />
          </div>
          <div className="flex flex-col gap-2">
            <Label htmlFor="s-url">URL</Label>
            <Input id="s-url" defaultValue="https://api.nzbgeek.info" />
          </div>
        </div>
        <SheetFooter>
          <Button>Save</Button>
          <Button variant="outline">Cancel</Button>
        </SheetFooter>
      </SheetContent>
    </Sheet>
  );
}
