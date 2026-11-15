import { defineConfig } from "vite";

// In a browser (`npm run dev`), /api is proxied to `swiftwin --mock ui --port 8787`.
export default defineConfig({
  clearScreen: false,
  server: { port: 5173, strictPort: true, proxy: { "/api": "http://127.0.0.1:8787", "/token": "http://127.0.0.1:8787" } },
  build: { target: "es2021", outDir: "dist", chunkSizeWarningLimit: 8000 },
});
