# glisse (vendored)

[glisse](https://github.com/0kzh/glisse) by Kelvin Zhang: Apple's iOS 26 Liquid Glass for the web.
MIT License, see `LICENSE`.

Vendored from commit `46be5ba07b60eb507e887ff338ded7840576a79f` (version 0.5.0). The npm package only
exports React components; SwiftWin's UI is plain JavaScript, so it uses glisse's framework-free core directly:

| File | Change |
|---|---|
| `lens.ts`, `svg.ts`, `spring.ts`, `styles.ts` | unchanged (`src/internal/`) |
| `refraction.ts` | the `useRefraction` hook became `attachRefraction(el, { bendsBackdrop })`, the same body as its effect |
| `press.ts` | the `usePress` hook became `attachPress(el)`, the same body as its effect |
| `styles.css` | lines 1–227 of `src/styles.css`: the theme variables, `.lg-glass` and `.lg-button` |
| `index.ts` | SwiftWin's own: `glass(el, { bend, press })` and `autoGlass()` for `[data-glass]` elements |

To update: copy the same files from a newer commit and redo the two small edits above.
