import { defineConfig } from "vitest/config";

// Vitest config for urdf-robo-dresser-vscode. Property, example, and
// integration tests live under test/, split by kind (mirroring the
// unified-kiro-extension test layout), separate from the source in src/.
export default defineConfig({
  test: {
    include: ["test/**/*.test.ts", "src/**/*.test.ts"],
    environment: "node",
  },
});
