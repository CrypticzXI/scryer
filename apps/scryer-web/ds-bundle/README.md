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

# ScryerWeb (scryer-web@0.1.0)

This design system is the published scryer-web React library, bundled as a single
browser global. All 23 components are the real upstream code.

## Where things are

- `_ds_bundle.js` — the whole-DS bundle at the project root; loads every component to `window.ScryerWeb`. First line is a `/* @ds-bundle: … */` metadata header.
- `styles.css` — the single stylesheet entry: it `@import`s the tokens, fonts, and component styles (`_ds_bundle.css`). Link this one file.
- `components/<group>/<Name>/<Name>.prompt.md` (example JSX + variants), `<Name>.d.ts` (types), `<Name>.html` (variant grid).
- `tokens/*.css` — CSS custom properties, names verbatim from upstream.
- `fonts/` — `@font-face` files + `fonts.css` (when the package ships fonts).

For a specific component, `read_file("components/<group>/<Name>/<Name>.prompt.md")`.

## Loading

Add these two lines to your page once (React must be on the page first):

```html
<link rel="stylesheet" href="styles.css">
<script src="_ds_bundle.js"></script>
```

Components are then available at `window.ScryerWeb.*`. Mount into a dedicated child node (e.g. `<div id="ds-root">`), not the host page's own React root, so the two trees don't collide:

```jsx
const { Button } = window.ScryerWeb;
ReactDOM.createRoot(document.getElementById('ds-root')).render(<Button />);
```

## Tokens

333 CSS custom properties from scryer-web. Names are
preserved verbatim from upstream. They are declared inside `_ds_bundle.css` (this DS ships one compiled stylesheet rather than separate token files).

- **color** (166): `--color-red-50`, `--color-red-100`, `--color-red-200`, …
- **spacing** (7): `--tw-space-y-reverse`, `--tw-space-x-reverse`, `--tw-ring-inset`, …
- **typography** (17): `--font-sans`, `--font-mono`, `--font-weight-normal`, …
- **radius** (3): `--radius-xs`, `--radius-xl`, `--radius`
- **shadow** (9): `--drop-shadow-md`, `--tw-shadow`, `--tw-ring-shadow`, …
- **other** (131): `--spacing`, `--container-xs`, `--container-sm`, …

## Components

### general
- `Button`
- `Card`
- `Checkbox`
- `Collapsible`
- `Command`
- `Dialog`
- `HoverCard`
- `Input`
- `Label`
- `Popover`
- `Progress`
- `Select`
- `Separator`
- `Sheet`
- `Sidebar`
- `Skeleton`
- `Table`
- `Tabs`
- `Textarea`
- `TimePicker`
- `Toaster`
- `ToggleGroup`
- `Tooltip`
