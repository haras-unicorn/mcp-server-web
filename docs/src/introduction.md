# Introduction

`mcp-server-web` is an [MCP](https://modelcontextprotocol.io) server that gives
an agent two tools:

- `web_fetch` fetches a URL and returns the main content as Markdown.
- `web_search` searches the web through a SearXNG compatible engine.

It talks over stdio and is configured with environment variables.

## Get it

With Nix:

```sh
nix build github:haras-unicorn/mcp-server-web
```

The binary is `result/bin/mcp-server-web`. Without Nix, download
`mcp-server-web-<system>` from the
[releases](https://github.com/haras-unicorn/mcp-server-web/releases). It is a
single static binary on Linux.

## Next

- Use it: [connect it to an agent](connect.md), then
  [configure it](configuration.md).
- Change it: see [where things are](development.md).
