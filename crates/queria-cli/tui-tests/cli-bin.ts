import fs from "node:fs";
import path from "node:path";

/**
 * Absolute path to the workspace debug queria-cli.
 */
export function queriaCliBin(): string {
  if (process.env.QUERIA_CLI_BIN) {
    const p = path.resolve(process.env.QUERIA_CLI_BIN);
    if (!fs.existsSync(p)) {
      throw new Error(`QUERIA_CLI_BIN not found: ${p}`);
    }
    return p;
  }

  let dir = process.cwd();
  for (let i = 0; i < 8; i++) {
    for (const candidate of [
      path.join(dir, "target", "debug", "queria-cli"),
      path.join(dir, "..", "..", "..", "target", "debug", "queria-cli"),
    ]) {
      if (fs.existsSync(candidate)) {
        return path.resolve(candidate);
      }
    }
    const parent = path.dirname(dir);
    if (parent === dir) break;
    dir = parent;
  }

  throw new Error(
    "queria-cli binary not found (run cargo build -p queria-cli, or set QUERIA_CLI_BIN)",
  );
}

/**
 * Put the debug binary directory first on the *worker* process PATH.
 *
 * `@factory/tui-test` resolves program.file via npm `which`, which consults
 * `process.env.PATH` of the runner (not the pty spawn env). Brew/home installs
 * at /usr/local/bin/queria-cli otherwise shadow the feature-branch binary.
 */
export function pinDebugCliOnWorkerPath(): string {
  const bin = queriaCliBin();
  const binDir = path.dirname(bin);
  const pathKey = process.platform === "win32" ? "Path" : "PATH";
  const existing = process.env[pathKey] || process.env.PATH || "";
  const next = `${binDir}${path.delimiter}${existing}`;
  process.env[pathKey] = next;
  process.env.PATH = next;
  return bin;
}

/** Env for the pty: isolated config + no agent token + CLI dir on PATH. */
export function isolatedPtyEnv(
  base: { [key: string]: string | undefined } | undefined,
  cfg: string,
  home: string,
): { [key: string]: string | undefined } {
  const binDir = path.dirname(queriaCliBin());
  const pathKey = process.platform === "win32" ? "Path" : "PATH";
  const existing =
    (base && base[pathKey]) || process.env[pathKey] || process.env.PATH || "";
  return {
    ...base,
    [pathKey]: `${binDir}${path.delimiter}${existing}`,
    QUERIA_CONFIG: cfg,
    QUERIA_AGENT_TOKEN: "",
    HOME: home,
  };
}
