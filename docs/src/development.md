# Where things are

```text
flake.nix                      entry point, calls haraspkgs makeFlake over ./src
Cargo.toml, Cargo.lock         Rust workspace
src/crates/mcp-server-web/
  src/main.rs                  stdio entry point
  src/config.rs                environment settings and the domain policy
  src/server.rs                the tools, fetching, searching
  tests/web.rs                 integration tests against a fake local server
src/nix/
  systems.nix                  supported systems
  packages/mcp-server-web.nix         the static package (crane, musl)
  packages/docs.nix            this book
  dev/shell.nix                dev shell, checks, formatters, CI workflows
docs/                          this book (mdBook)
```

## Work on it

```sh
nix develop      # toolchain, formatters, checks
dev test         # cargo test
dev lint         # clippy
dev format       # format everything
dev check        # spell check, markdown lint, nix lint, static link check
dev docs         # build this book
nix flake check  # all checks, as CI runs them
nix build        # the static binary
```

Cargo works offline in the shell: it points at crates that Nix vendored from
`Cargo.lock`. After changing dependencies, update `Cargo.lock` with network
access and enter the shell again.

## Generated files

Entering the shell writes `.gitignore`, `.gitattributes`, `LICENSE.md`,
`CODE_OF_CONDUCT.md`, `.github/workflows/*` and `.cargo/config.toml`. Their
source is `src/nix/dev/shell.nix`. Change it there, not in the files.

## Tests

Tests never use the network. Unit tests sit next to the code. Integration tests
in `tests/web.rs` start a small HTTP server on localhost and call the server
against it. The Nix sandbox has no certificate store, so the package build sets
`SSL_CERT_FILE` and the dev shell sets `SSL_CERT_DIR`, both from `pkgs.cacert`
(`nix develop` drops `SSL_CERT_FILE`).
