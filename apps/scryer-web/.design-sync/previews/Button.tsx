// Authored preview for the Scryer <Button>. Each named export is one card cell.
// Realistic media-manager content (Add Movie / Search / Refresh / Delete …).
import { Button } from "scryer-web";

const Plus = () => (
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
    <path d="M5 12h14M12 5v14" />
  </svg>
);
const Search = () => (
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
    <circle cx="11" cy="11" r="8" /><path d="m21 21-4.3-4.3" />
  </svg>
);
const Refresh = () => (
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
    <path d="M3 12a9 9 0 0 1 15-6.7L21 8" /><path d="M21 3v5h-5" /><path d="M21 12a9 9 0 0 1-15 6.7L3 16" /><path d="M3 21v-5h5" />
  </svg>
);

export function Variants() {
  return (
    <div className="flex flex-wrap items-center gap-3 p-6">
      <Button>Add Movie</Button>
      <Button variant="secondary">Refresh</Button>
      <Button variant="outline">Search</Button>
      <Button variant="ghost">Cancel</Button>
      <Button variant="destructive">Delete</Button>
      <Button variant="link">View details</Button>
    </div>
  );
}

export function Sizes() {
  return (
    <div className="flex flex-wrap items-center gap-3 p-6">
      <Button size="sm">Small</Button>
      <Button size="default">Default</Button>
      <Button size="lg">Large</Button>
    </div>
  );
}

export function WithIcon() {
  return (
    <div className="flex flex-wrap items-center gap-3 p-6">
      <Button><Plus />Add Movie</Button>
      <Button variant="outline"><Search />Search Indexers</Button>
      <Button variant="secondary"><Refresh />Refresh Library</Button>
    </div>
  );
}

export function IconButtons() {
  return (
    <div className="flex flex-wrap items-center gap-3 p-6">
      <Button size="icon" aria-label="Add"><Plus /></Button>
      <Button size="icon" variant="outline" aria-label="Search"><Search /></Button>
      <Button size="icon" variant="ghost" aria-label="Refresh"><Refresh /></Button>
    </div>
  );
}

export function States() {
  return (
    <div className="flex flex-wrap items-center gap-3 p-6">
      <Button>Enabled</Button>
      <Button disabled>Disabled</Button>
      <Button variant="outline" disabled>Unavailable</Button>
    </div>
  );
}
