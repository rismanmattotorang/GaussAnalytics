import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// During development, API calls are proxied to the Rust server (gaussctl serve).
// In production, the server hosts these built assets directly.
export default defineConfig({
  plugins: [react()],
  server: {
    port: 5173,
    proxy: {
      "/api": "http://127.0.0.1:3000",
    },
  },
  build: {
    outDir: "dist",
    rollupOptions: {
      output: {
        // Split the charting stack (nivo + d3) into its own cacheable chunk so
        // the app shell stays small and the heavy viz code is fetched once.
        manualChunks: {
          charts: ["@nivo/core", "@nivo/bar", "@nivo/line", "@nivo/pie", "@nivo/scatterplot"],
        },
      },
    },
  },
});
