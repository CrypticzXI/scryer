// Authored preview for the Scryer <Input>. State-heavy: default / filled / disabled / invalid,
// plus a labelled field (composed with <Label>). Realistic media-manager placeholders.
import { Input, Label } from "scryer-web";

export function States() {
  return (
    <div className="flex flex-col gap-3 p-6 max-w-sm">
      <Input placeholder="My Indexer" />
      <Input defaultValue="http://localhost:8989" />
      <Input placeholder="Disabled field" disabled />
      <Input defaultValue="not-a-valid-url" aria-invalid />
    </div>
  );
}

export function WithLabel() {
  return (
    <div className="flex flex-col gap-2 p-6 max-w-sm">
      <Label htmlFor="indexer-url">Indexer URL</Label>
      <Input id="indexer-url" placeholder="https://indexer.example.com" />
      <p className="text-xs text-muted-foreground">
        The base URL Scryer uses to query this indexer.
      </p>
    </div>
  );
}

export function SearchField() {
  return (
    <div className="flex flex-col gap-3 p-6 max-w-sm">
      <Input placeholder="filter…" />
      <Input type="search" placeholder="Search movies and series" />
    </div>
  );
}
