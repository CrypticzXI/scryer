## Scryer Design System — how to build with it

A Sonarr/Radarr-style media-manager UI built on **shadcn/Radix primitives + Tailwind v4**.
All components are on `window.ScryerWeb.*` (imported from the bound bundle). The 23 cards are
the headline components; their compound subparts are also exports (e.g. `DialogContent`,
`SelectItem`, `CardHeader`, `TableRow`, `SidebarMenuButton`).

### Setup & theming
- **No ThemeProvider** — theming is class-based. The default (`:root`) is the **light** theme;
  add `class="dark"` (or `class="pride"`) to an ancestor (e.g. `<html>`) for the dark/pride theme.
  Tokens (`--background`, `--primary`, …) are defined per theme in the stylesheet.
- **Providers that ARE required**: wrap tooltips in `<TooltipProvider>`; wrap any `Sidebar*`
  usage in `<SidebarProvider>`. Render `<Toaster />` once near the app root and fire toasts
  imperatively with `toast()` / `toast.success()` / `toast.error()`.
- Most other components (Button, Card, Input, Dialog, Select, Tabs, Table, …) need no wrapper.

### Styling idiom — Tailwind utilities bound to design tokens
Style with Tailwind classes; **prefer the token-backed semantic utilities over raw colors** so
output tracks the theme. Real names from the stylesheet:

| Purpose | Classes |
|---|---|
| Surfaces | `bg-background`, `bg-card`, `bg-popover`, `bg-muted`, `bg-accent`, `bg-sidebar`, `bg-field` (form inputs) |
| Text | `text-foreground`, `text-muted-foreground`, `text-card-foreground`, `text-primary-foreground` |
| Brand / intent | `bg-primary` (indigo), `bg-secondary`, `bg-destructive` + `text-destructive` (red) |
| Borders / focus | `border` + `border-border`, `border-input`, focus rings `focus-visible:ring-ring/50` |
| Radii | `rounded-md` / `rounded-lg` / `rounded-xl` (scale from `--radius`) |

**Variants are props, not classes.** e.g. `Button` takes `variant="default|primary|secondary|
outline|ghost|destructive|link"` and `size="default|sm|lg|icon"`; `ToggleGroup`/`Tabs` similar.
Reach for utilities only for layout glue (`flex`, `grid`, `gap-*`, `p-*`, `max-w-*`).

**Fonts** are automatic: body/UI = Inter, headings `h1`–`h6` = Space Grotesk, code (`[data-code-font]`)
= JetBrains Mono. Don't set font-family by hand.

**Compose compound components** from their parts: `Card`+`CardHeader`+`CardTitle`+`CardContent`;
`Dialog`+`DialogContent`+`DialogHeader`+`DialogTitle`+`DialogDescription`+`DialogFooter`;
`Select`+`SelectTrigger`+`SelectValue`+`SelectContent`+`SelectItem`; `Table`+`TableHeader`+
`TableBody`+`TableRow`+`TableHead`+`TableCell`.

### Where the truth is
Read the bound `styles.css` (and its `@import`ed `_ds_bundle.css` + `fonts/`) for the exact token
and utility set, and each component's `<Name>.d.ts` + `<Name>.prompt.md` for its API and examples.

### Idiomatic example
```tsx
import { Card, CardHeader, CardTitle, CardContent, Button } from "scryer-web";

<Card>
  <CardHeader><CardTitle>Download Client</CardTitle></CardHeader>
  <CardContent>
    <p className="text-sm text-muted-foreground">qBittorrent · connected</p>
    <div className="flex items-center gap-2">
      <Button size="sm">Test</Button>
      <Button size="sm" variant="outline">Edit</Button>
      <Button size="sm" variant="destructive">Remove</Button>
    </div>
  </CardContent>
</Card>
```
