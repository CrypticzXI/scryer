# design-sync notes — scryer-web

Repo-specific gotchas for syncing `scryer-web`'s `components/ui` (shadcn/Radix, Tailwind v4)
to claude.ai/design. Read this before re-syncing.

## Build shape

- **scryer-web is a Vite *app*, not a published library** — no `module`/`main`/`exports`, no
  library `dist/`. We build via a **barrel + synth bundle**:
  - `cfg.entry = .design-sync/ds-entry.ts` re-exports every `components/ui/*` file, so esbuild
    assembles the full `window.ScryerWeb` export surface (~109 symbols).
  - `cfg.componentSrcMap` enumerates the **23 primary components** that surface as cards (Button,
    Card, Dialog, Select, Table, Tabs, Sidebar, Tooltip, …). Compound subparts (CardHeader,
    DialogContent, SelectItem, …) stay importable in the bundle but are not separate cards.
  - Passing `cfg.entry` makes the converter resolve `PKG_DIR` by walking up from the barrel, which
    avoids the own-repo crash (`node_modules/scryer-web/package.json` ENOENT) that the no-entry
    synth path hits. **Do NOT remove `cfg.entry`** or you'll need a `node_modules/scryer-web`
    self-symlink instead.
- **`.d.ts` contracts are thin** (`{[key:string]:unknown}`) because there are no compiled types.
  The API is conveyed via authored previews + `.prompt.md` + the conventions header. A future
  `tsc --emitDeclarationOnly` library build would give full per-component prop types — worth doing.

## CSS + fonts

- `cfg.buildCmd = node .design-sync/build-css.mjs` compiles `app/globals.css` (Tailwind v4) into
  `.design-sync/.cache/ds.css` with `@source` globs covering `components/`, `src/`, and the authored
  previews — so every utility a component or preview uses is present. `cfg.cssEntry` points at it.
  **Always runs before the converter** (the driver runs buildCmd first).
- Fonts: `cfg.extraFonts = .design-sync/fonts.css` ships the real `@fontsource-variable` brand fonts
  (Inter, Space Grotesk, JetBrains Mono — latin + latin-ext weight-axis subsets only, to stay lean).

## Known render warns (triaged — not new issues)

- **`[FONT_MISSING]` "Cascadia Code" / "Source Code Pro"**: these are *system fallbacks* in the
  `--font-code` stack in globals.css, never meant to ship. The brand mono (JetBrains Mono Variable)
  IS shipped. Benign — ignore.
- **`Toaster` `[RENDER_THIN]` / `maxHeight=0`**: sonner renders toasts into a `document.body`
  portal, so the card *root* measures 0 height even though the toasts render correctly (verified in
  the review sheet — three expanded toasts: default, red error, green success). Expected, not blank.

## Render check / playwright (IMPORTANT)

- **Playwright's browser *extraction* hangs in this environment** (the 170MB chromium zip downloads
  fine in seconds, but `npx playwright install chromium` then hangs forever unpacking it; sandbox
  on/off makes no difference). Workaround used: let the download finish, then extract the temp zip
  manually with `ditto -x -k <temp.zip> ~/.cache/ms-playwright/chromium-1208` and `touch
  ~/.cache/ms-playwright/chromium-1208/INSTALLATION_COMPLETE`. Extraction via ditto is <1s.
- Playwright 1.58 `chromium.launch({headless})` defaults to the **headless-shell** browser (not
  installed). We use the full **Chrome for Testing** binary instead via the env var the scripts honor:
  `export DS_CHROMIUM_PATH="$HOME/.cache/ms-playwright/chromium-1208/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing"`
  **Set `DS_CHROMIUM_PATH` for every validate / capture / preview-rebuild run (and in subagent
  prompts).** Without it, those scripts try to launch the missing headless-shell and re-trigger the
  hanging download.

## Re-sync risks

- The chromium install above is **machine state** (gitignored, not in the bundle). A fresh machine
  must redo the ditto-extract trick before the render check will run.
- `cfg.componentSrcMap` is a hand-maintained list of 23 primaries — if `components/ui/` gains a new
  primitive, add it to both `ds-entry.ts` (regenerate) and `componentSrcMap`.
- `.d.ts` are stubs by design (see above); don't mistake that for a regression.
