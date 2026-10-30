# Make a release

Releases are automated with [release-plz](https://release-plz.dev).

## One time setup

Add three repository secrets:

- `RELEASE_PLZ_TOKEN`: a personal access token that can write contents and pull
  requests. It must not be the default `GITHUB_TOKEN`, because tags and releases
  made with that token do not start the `build` workflow.
- `CARGO_REGISTRY_TOKEN`: a crates.io token that can publish `mcp-server-web`.
- `CACHIX_AUTH_TOKEN`: a Cachix auth token, so the `check` and `build`
  workflows can push to the cache. Without it they still run, but push nothing.

## Release

1. Merge changes to `main` with conventional commit messages.
2. The `release` workflow opens a pull request that bumps the version and the
   changelog. Merge it.
3. The `release` workflow then publishes to crates.io and creates the tag and
   the GitHub release.
4. The published release starts the `build` workflow, which builds with Nix on
   each system and uploads `mcp-server-web-<system>` to the release.
