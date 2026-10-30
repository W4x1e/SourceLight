/// <reference types="vite/client" />

// Vite's ambient types — `import.meta.env`, and the asset import suffixes such
// as `?raw` and `?url`. The project had no such file, so those imports had no
// declarations; `styles.test.ts` reads the stylesheet through `?raw` to check
// the colour ramp's contrast without pulling in Node types for a browser app.
