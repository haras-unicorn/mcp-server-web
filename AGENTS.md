# mcp-server-web

A Rust MCP server over stdio with two tools, `web_fetch` and `web_search`. It is
configured only through environment variables. The Nix flake is built on
haraspkgs (`makeFlake`). See `README.md` for usage and `docs/` for the book.

## Structure

- `flake.nix`: entry point. It hands `./src` to haraspkgs `makeFlake`.
- `src/crates/`: the Cargo workspace members. Unit tests sit next to the code,
  integration tests are in each crate's `tests/` and use a fake local server.
- `src/nix/packages/`: Nix packages (the static binary built with crane, the
  docs book).
- `src/nix/dev/shell.nix`: the dev shell, commands, checks, formatters and CI
  workflows.
- `docs/`: the mdBook, split into using the server and developing it.

## Development

Enter the dev shell with `nix develop` (direnv does it through `.envrc`). It
provides `dev` subcommands for testing, linting, formatting, checking and
building the docs. `dev --help` lists them. `nix flake check` runs everything CI
runs. Cargo works offline in the shell against crates vendored by Nix, so after
changing dependencies update `Cargo.lock` with network access.

Tests never use the network.

## Generated files

Entering the dev shell writes `.gitignore`, `.gitattributes`, `LICENSE.md`,
`CODE_OF_CONDUCT.md`, `.github/workflows/*` and `.cargo/config.toml`. Change
them in `src/nix/dev/shell.nix`, not in the files. Workflow behavior that is not
set there comes from haraspkgs.

## Releases

Commits follow Conventional Commits, because release-plz builds versions and
changelogs from them. Merging its release PR publishes to crates.io and creates
the GitHub release, which starts the workflow that uploads binaries. The secrets
it needs are listed in `docs/src/releasing.md`.
