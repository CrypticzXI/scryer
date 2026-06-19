// Authored preview for the Scryer <Command> palette (cmdk; renders inline).
import { Command, CommandInput, CommandList, CommandGroup, CommandItem, CommandSeparator, CommandShortcut, CommandEmpty } from "scryer-web";

export function Palette() {
  return (
    <div className="p-6">
      <Command className="max-w-md rounded-lg border border-border shadow-md">
        <CommandInput placeholder="Search movies, series, settings…" />
        <CommandList>
          <CommandEmpty>No results found.</CommandEmpty>
          <CommandGroup heading="Library">
            <CommandItem>Add movie<CommandShortcut>⌘A</CommandShortcut></CommandItem>
            <CommandItem>Search wanted</CommandItem>
            <CommandItem>Refresh all</CommandItem>
          </CommandGroup>
          <CommandSeparator />
          <CommandGroup heading="Go to">
            <CommandItem>Activity</CommandItem>
            <CommandItem>Calendar</CommandItem>
            <CommandItem>Settings<CommandShortcut>⌘,</CommandShortcut></CommandItem>
          </CommandGroup>
        </CommandList>
      </Command>
    </div>
  );
}
