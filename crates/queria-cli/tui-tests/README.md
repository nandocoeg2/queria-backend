# queria-cli hub TUI smoke tests

PTY-level smoke tests for `queria-cli tui` using
[@factory/tui-test](https://github.com/Factory-AI/tui-test)
(`github:Factory-AI/tui-test`).

## Requirements

- Node.js 18–24 (Node 26+ fails the package engines range)
- workspace debug binary: `cargo build -p queria-cli`
- native PTY (`node-pty`) via npm install

## Run

```bash
cd crates/queria-cli/tui-tests
npm install
npm test
```

`pretest` rebuilds `queria-cli`. `QUERIA_CLI_BIN` is set to the workspace
`target/debug/queria-cli` so a brew install (`/usr/local/bin/queria-cli`) is
not used — tui-test resolves `program.file` via `which` on the worker PATH.

## Coverage

| Test | Asserts |
|------|---------|
| hub menu | Doctor / Index / Status / Config / Quit |
| doctor | `d` opens doctor, Esc returns |
| status soft-degrade | no token → Status error, hub not ejected |
| index wizard | `i` opens index-here wizard |
| quit | `q` leaves hub |

Traces: `npm run test:trace` (writes under `tui-traces/`).
