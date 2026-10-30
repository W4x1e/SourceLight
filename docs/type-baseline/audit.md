# Typography baseline

Every main surface at the minimum window (940x620) and the default (1500x940),
with the screenshot of each beside this file as the baseline for the next type
change, and a check for text the layout clips.

**How these were made (2026-10-11).** From the browser harness, not the real
app: `npm run dev` serves the interface to a plain browser backed by
`src/dev/mockBackend.ts`, whose fake 48-photo library shows generated
placeholder images ("Looking at the UI without Tauri" in CONTRIBUTING.md).
Microsoft Edge, driven by playwright-core, opened each surface the way
`scripts/audit-webview.mjs` does, ran the same check (`src/dev/clipAudit.ts`,
which the harness installs as `__clipAudit()`) and took the screenshot. The
earlier baseline came from a release build showing a real photo library, and
was replaced so that no personal photographs are kept in the repository.

Keep it that way: the screenshots are committed, so regenerate them from the
harness, or from the real app (`scripts/audit-webview.mjs`) pointed at a
throwaway library, never at your own photos.

Two differences from the first baseline. Settings → Advanced is now
Settings → Library; its files keep the `settings-advanced` name. And the first
baseline's 940x620 shots of every surface after the Lightbox were taken at
1461x903 (the window had grown back); these are all at the size their names
say.

Rendered fonts: UI text → IBM Plex Sans Var; display heading → IBM Plex Sans Var

| Surface | Size | Clipped | Unreachable | Ellipsised (deliberate) |
| --- | --- | --: | --: | --: |
| Browse: grid and filter rail | 940x620 | 1 | 0 | 4 |
| Browse: a photo selected | 940x620 | 1 | 0 | 1 |
| Lightbox and filmstrip | 940x620 | 1 | 0 | 1 |
| Cull: loupe | 940x620 | 1 | 0 | 0 |
| Cull: survey | 940x620 | 1 | 0 | 0 |
| Develop | 940x620 | 1 | 0 | 1 |
| Develop: masking | 940x620 | 1 | 0 | 1 |
| Export dialog | 940x620 | 1 | 0 | 1 |
| Settings: AI | 940x620 | 1 | 0 | 2 |
| Settings: Library | 940x620 | 1 | 0 | 1 |
| Settings: Export | 940x620 | 1 | 0 | 1 |
| Settings: About | 940x620 | 1 | 0 | 1 |
| Keyboard shortcuts | 940x620 | 1 | 0 | 1 |
| Places | 940x620 | 0 | 0 | 2 |
| People | 940x620 | 0 | 0 | 0 |
| Browse: grid and filter rail | 1500x940 | 1 | 0 | 4 |
| Browse: a photo selected | 1500x940 | 1 | 0 | 4 |
| Lightbox and filmstrip | 1500x940 | 1 | 0 | 4 |
| Cull: loupe | 1500x940 | 1 | 0 | 0 |
| Cull: survey | 1500x940 | 1 | 0 | 0 |
| Develop | 1500x940 | 1 | 0 | 4 |
| Develop: masking | 1500x940 | 1 | 0 | 4 |
| Export dialog | 1500x940 | 1 | 0 | 4 |
| Settings: AI | 1500x940 | 1 | 0 | 5 |
| Settings: Library | 1500x940 | 1 | 0 | 4 |
| Settings: Export | 1500x940 | 1 | 0 | 4 |
| Settings: About | 1500x940 | 1 | 0 | 4 |
| Keyboard shortcuts | 1500x940 | 1 | 0 | 4 |
| Places | 1500x940 | 0 | 0 | 0 |
| People | 1500x940 | 0 | 0 | 0 |

No visible text is clipped and no control is out of reach. The one clipped
element counted on most surfaces is the timeline's keyboard help for screen
readers (`p.sr-only`), which is hidden from sight on purpose.
