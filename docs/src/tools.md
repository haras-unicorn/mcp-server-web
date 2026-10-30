# Tools

A failed call is not a protocol error. The result has `isError` set and the text
says what went wrong, so the agent can react.

## `web_fetch`

| Input       | Type    | Meaning                                        |
| ----------- | ------- | ---------------------------------------------- |
| `url`       | string  | Required. An `http` or `https` URL.            |
| `max_chars` | integer | Longest result in characters. Default `20000`. |

Output:

- HTML: the main content as Markdown, under a `# <title>` heading. If no article
  is found, the page as is.
- Text, JSON, XML, YAML, JavaScript: the body as is.
- Anything else: an error.

Output longer than `max_chars` is cut and ends with
`[truncated, <n> more characters]`. Up to 10 redirects are followed.

Errors: `is not allowed by the domain policy`, `invalid url`,
`responded with <status>`, `unsupported content type`,
`is larger than <n> bytes`, `request to <url> failed`.

## `web_search`

| Input   | Type    | Meaning                               |
| ------- | ------- | ------------------------------------- |
| `query` | string  | Required. The search query.           |
| `limit` | integer | Most results to return. Default `10`. |

Output is a numbered list, or `No results.`:

```text
1. A title
   https://one.example/a
   A snippet
```

Results on refused domains are dropped before `limit` is applied.

Errors: `web_search is not configured`, `invalid MCP_WEB_SEARCH_URL`,
`search engine responded with <status>`, `did not return SearXNG style JSON`.
