import { defineConfig } from "vitest/config";

// Vitest config for the extension host's unit/property/integration tests.
// These plain CommonJS/ESM modules run in a Node environment; the VS Code
// globals they touch are mocked per-test.
export default defineConfig({
  test: {
    include: ["src/**/*.test.ts", "test/**/*.test.ts"],
    environment: "node",
  },
});
