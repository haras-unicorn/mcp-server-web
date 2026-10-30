# Connect to an agent

Add `mcp-server-web` to your agent as a stdio server. Settings go in `env`; see
[Configuration](configuration.md).

Most agents take JSON like this:

```json
{
  "mcpServers": {
    "web": {
      "command": "/path/to/mcp-server-web",
      "env": {
        "MCP_WEB_SEARCH_URL": "https://searx.example/search"
      }
    }
  }
}
```

With Claude Code:

```sh
claude mcp add web -e MCP_WEB_SEARCH_URL=https://searx.example/search -- /path/to/mcp-server-web
```

The agent should now list `web_fetch` and `web_search`. Logs go to stderr; set
`RUST_LOG=debug` to see more.
