# Add a feature or fix a bug

## Fix a bug

1. Reproduce it as a test. For fetching and searching, add a case to
   `src/crates/mcp-server-web/tests/web.rs`: `serve(...)` starts a fake server
   with canned responses, and `server(...)` builds a `WebServer` on top of it.
   For pure logic, add a unit test next to the code.
2. Fix it, and run `dev test` and `dev lint`.
3. Commit with a `fix:` message.

## Add a setting

1. In `src/config.rs`, add the variable name constant and a field on `Config`,
   and read it in `Config::from_env`.
2. Use it in `src/server.rs`. If it changes what is allowed, `DomainPolicy` in
   `src/config.rs` is where that logic lives.
3. Add a test, and a row to [Configuration](configuration.md).
4. Commit with a `feat:` message.

## Add a tool

1. In `src/server.rs`, add a request struct that derives `Deserialize` and
   `schemars::JsonSchema`.
2. Put the logic in a `pub async fn` on `WebServer` that returns
   `anyhow::Result<String>`, so tests can call it directly.
3. Add a thin method in the `#[tool_router]` block with
   `#[tool(description = "...")]`, which passes the result through
   `into_result`.
4. Add tests, and a section to [Tools](tools.md).
5. Commit with a `feat:` message.

## Before you push

Run `nix flake check`. Commit messages follow
[Conventional Commits](https://www.conventionalcommits.org), because releases
are made from them.
