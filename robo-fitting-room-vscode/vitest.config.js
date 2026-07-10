import { defineConfig } from "vitest/config";

// Vitest config for the webview (media/js) unit tests. These plain ESM
// modules run in a Node environment; the VS Code / browser globals they
// touch (acquireVsCodeApi, FileReader) are mocked per-test.
export default defineConfig({
  test: {
    include: ["media/**/*.test.js", "src/**/*.test.ts"],
    environment: "node",
  },
});
