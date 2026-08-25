import { defineConfig } from "vite";

export default defineConfig({
  root: "viewer",
  build: {
    outDir: "../dist",
    emptyOutDir: true,
    sourcemap: false,
  },
});
