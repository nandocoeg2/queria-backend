/**
 * Hub TUI smoke tests via @factory/tui-test
 * (https://github.com/Factory-AI/tui-test).
 *
 * Spawns the workspace debug queria-cli with an isolated config and no real
 * edge (soft-degrade Status / Doctor Fail paths).
 */
import { test, expect } from "@factory/tui-test";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { isolatedPtyEnv, pinDebugCliOnWorkerPath } from "../cli-bin.js";

function isolatedConfigPath(): string {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "queria-tui-"));
  const cfg = path.join(dir, "config.toml");
  // Valid shape, no token — laptop soft-degrade / doctor Fail paths.
  fs.writeFileSync(
    cfg,
    `[profiles.tui-smoke]
edge_url = "http://127.0.0.1:1"
`,
    "utf8",
  );
  return cfg;
}

test.beforeSpawn(async (options) => {
  // Must pin worker PATH before spawn: tui-test uses which(process.env.PATH).
  pinDebugCliOnWorkerPath();
  const cfg = isolatedConfigPath();
  const home = fs.mkdtempSync(path.join(os.tmpdir(), "queria-home-"));
  return {
    ...options,
    env: isolatedPtyEnv(
      { ...process.env, ...options.env },
      cfg,
      home,
    ),
    program: {
      file: "queria-cli",
      args: ["tui", "--profile", "tui-smoke"],
    },
  };
});

test("hub menu renders Doctor Index Status Config", async ({ terminal }) => {
  await expect(terminal.getByText("queria-cli hub")).toBeVisible();
  await expect(terminal.getByText("[d] Doctor")).toBeVisible();
  await expect(terminal.getByText("[i] Index")).toBeVisible();
  await expect(terminal.getByText("[s] Status")).toBeVisible();
  await expect(terminal.getByText("[c] Config")).toBeVisible();
  await expect(terminal.getByText("[q] Quit")).toBeVisible();
});

test("d opens doctor screen and Esc returns to hub", async ({ terminal }) => {
  await expect(terminal.getByText("queria-cli hub")).toBeVisible();
  terminal.write("d");
  await expect(terminal.getByText(" doctor ")).toBeVisible({ timeout: 20_000 });
  // Unique row labels (PASS/FAIL alone appear on many rows → strict mode fail).
  await expect(terminal.getByText("r re-run")).toBeVisible();
  await expect(terminal.getByText("Esc/q back")).toBeVisible();
  terminal.keyEscape();
  await expect(terminal.getByText("queria-cli hub")).toBeVisible();
  await expect(terminal.getByText("[d] Doctor")).toBeVisible();
});

test("s opens status screen soft-degrade without ejecting hub", async ({
  terminal,
}) => {
  await expect(terminal.getByText("queria-cli hub")).toBeVisible();
  terminal.write("s");
  await expect(terminal.getByText(" status ")).toBeVisible({ timeout: 15_000 });
  await expect(terminal.getByText("Status error")).toBeVisible();
  await expect(terminal.getByText("no agent token")).toBeVisible();
  terminal.keyEscape();
  await expect(terminal.getByText("queria-cli hub")).toBeVisible();
});

test("i opens index-here wizard", async ({ terminal }) => {
  await expect(terminal.getByText("queria-cli hub")).toBeVisible();
  terminal.write("i");
  await expect(terminal.getByText("index-here wizard")).toBeVisible({
    timeout: 30_000,
  });
  terminal.keyEscape();
  await expect(terminal.getByText("queria-cli hub")).toBeVisible({
    timeout: 20_000,
  });
});

test("q quits hub", async ({ terminal }) => {
  await expect(terminal.getByText("queria-cli hub")).toBeVisible();
  terminal.write("q");
  await new Promise((r) => setTimeout(r, 500));
});
