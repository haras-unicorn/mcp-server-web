# Configuration

Everything is an environment variable, read once at startup. An empty value
counts as unset.

| Variable                | Default                    | Meaning                          |
| ----------------------- | -------------------------- | -------------------------------- |
| `MCP_WEB_SEARCH_URL`    | unset                      | Search engine URL, see below.    |
| `MCP_WEB_ALLOW_DOMAINS` | unset (allow all)          | Domains that may be fetched.     |
| `MCP_WEB_DENY_DOMAINS`  | unset                      | Domains that are never fetched.  |
| `MCP_WEB_TIMEOUT_SECS`  | `30`                       | Timeout of each request.         |
| `MCP_WEB_MAX_BYTES`     | `5242880` (5 MiB)          | Largest body that is downloaded. |
| `MCP_WEB_USER_AGENT`    | `mcp-server-web/<version>` | `User-Agent` header.             |
| `RUST_LOG`              | errors only                | Log filter. Logs go to stderr.   |

A bad number in `MCP_WEB_TIMEOUT_SECS` or `MCP_WEB_MAX_BYTES` stops the server
at startup.

## Search engine

Without `MCP_WEB_SEARCH_URL`, `web_search` returns an error.

For SearXNG, enable the JSON format in `settings.yml` (`search.formats`) and set
the endpoint:

```sh
MCP_WEB_SEARCH_URL=https://searx.example/search
```

`q=<query>&format=json` is added to the URL. For another engine, put a `{query}`
placeholder where the URL-encoded query goes:

```sh
MCP_WEB_SEARCH_URL='https://engine.example/api?term={query}'
```

The engine must answer with `{"results": [{"title", "url", "content"}]}`. Only
`url` is required.

## Domain policy

`MCP_WEB_ALLOW_DOMAINS` and `MCP_WEB_DENY_DOMAINS` take entries separated by
commas or spaces.

- `example.com` matches `example.com` and all its subdomains, but not
  `notexample.com`. A leading `*.` is ignored.
- IP addresses only match exactly.
- A host on the denylist is refused, even when it is also on the allowlist.
- Otherwise a host is allowed when the allowlist is empty or the host is on it.
- Only `http` and `https` URLs are allowed.

The policy is applied to the requested URL, to every redirect, and to search
results. The search engine itself is exempt.

Local addresses are allowed by default. To block them:

```sh
MCP_WEB_DENY_DOMAINS=localhost,127.0.0.1
```

This matches names only. It is not a network boundary.
