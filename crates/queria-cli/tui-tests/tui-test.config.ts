import { defineConfig } from "@factory/tui-test";
import { pinDebugCliOnWorkerPath } from "./cli-bin.js";

// Config load happens in main process before workers; pin PATH so default
// program.file "queria-cli" resolves to the branch binary, not brew install.
pinDebugCliOnWorkerPath();

export default defineConfig({
  retries: process.env.CI ? 1 : 0,
  timeout: 45_000,
  expect: { timeout: 15_000 },
  workers: 1,
  testMatch: "tests/**/*.test.ts",
  use: {
    rows: 30,
    columns: 100,
    program: {
      file: "queria-cli",
      args: ["tui"],
    },
  },
});
