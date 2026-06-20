// Authored preview for the Scryer <Popover> — rendered open so the floating content shows.
import { Popover, PopoverTrigger, PopoverContent, PopoverHeader, PopoverTitle, PopoverDescription, Button, Label, Input } from "scryer-web";

export function Default() {
  return (
    <div className="flex justify-center pt-6">
      <Popover defaultOpen>
        <PopoverTrigger asChild>
          <Button variant="outline">Filters</Button>
        </PopoverTrigger>
        <PopoverContent>
          <PopoverHeader>
            <PopoverTitle>Quick filter</PopoverTitle>
            <PopoverDescription>Narrow the library view.</PopoverDescription>
          </PopoverHeader>
          <div className="mt-3 flex flex-col gap-2">
            <Label htmlFor="min-year">Released after</Label>
            <Input id="min-year" defaultValue="2010" />
          </div>
        </PopoverContent>
      </Popover>
    </div>
  );
}
