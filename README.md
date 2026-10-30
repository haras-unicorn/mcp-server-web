# mcp-server-web

An [MCP](https://modelcontextprotocol.io) server with two tools: `web_fetch`
returns a page as Markdown, and `web_search` queries a SearXNG compatible
engine. Domains can be limited with an allowlist and a denylist.

```sh
nix build github:haras-unicorn/mcp-server-web
MCP_WEB_SEARCH_URL=https://searx.example/search ./result/bin/mcp-server-web
```

Documentation is an mdBook in [`docs/`](docs/src/introduction.md). Build it with
`dev docs`.

## License

[MIT](LICENSE.md)
