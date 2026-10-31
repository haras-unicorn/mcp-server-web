# Tools

A failed call is not a protocol error. The result has `isError` set and the text
says what went wrong, so the agent can react.

## `web_fetch`

| Input    | Type    | Meaning                                                        |
| -------- | ------- | -------------------------------------------------------------- |
| `url`    | string  | Required. An `http` or `https` URL.                            |
| `mode`   | string  | `readable` (default) or `full`.                                |
| `offset` | integer | Character to start at. Default `0`.                            |
| `length` | integer | Characters to return. Default and maximum `MCP_WEB_MAX_CHARS`. |

Modes, for HTML pages:

- `readable`: the main content as Markdown, under a `# <title>` heading. If no
  article is found, the page as is.
- `full`: the page as is, HTML included.

Text, JSON, XML, YAML and JavaScript are returned as is in both modes. PNG,
JPEG, GIF and WebP images are returned whole, see below. Anything else is an
error.

Output for text is structured content, one slice of the result:

```json
{ "offset": 0, "length": 20000, "next_offset": 20000, "content": "..." }
```

`length` is what was returned, so it is shorter on the last slice, where
`next_offset` is `null`. To read a long page, repeat the call with the
`next_offset`. Offsets count characters of the result in the chosen mode, so
keep the mode the same between calls. Pages are cached, see
[Configuration](configuration.md#cache). Redirects are followed up to
`MCP_WEB_MAX_REDIRECTS`.

Errors: `is not allowed by the domain policy`, `invalid url`,
`responded with <status>`, `unsupported content type`,
`is larger than <n> bytes`, `request to <url> failed`,
`length <n> is larger than the maximum`, `offset <n> is past the end`,
`offset and length do not apply to images`, `failed to process the image`,
`pixels, more than the 50000000 that can be decoded`. An unknown `mode` is
refused by the schema.

### Images

An image is returned whole, as an image content block that the client hands to
the model as an image. The structured content describes it, and is also given as
text:

```json
{ "content_type": "image/jpeg", "bytes": 1234 }
```

An image with more than `MCP_WEB_MAX_IMAGE_PIXELS` pixels is scaled down to fit,
keeping its aspect ratio, and returned as JPEG. This keeps the result and the
cache small. Any other image is returned as it came. An animated image that is
scaled down keeps only its first frame, and an image with more than 50 million
pixels is refused, because that much would have to be decoded to scale it down.
`0` turns scaling off, and then nothing is decoded.

`mode` makes no difference, and `offset` and `length` are refused with an error.
Because an image comes with this mixed result, `web_fetch` declares no output
schema, unlike `web_search`.

## `web_search`

| Input   | Type    | Meaning                                                 |
| ------- | ------- | ------------------------------------------------------- |
| `query` | string  | Required. The search query.                             |
| `limit` | integer | Most results to return. Default `MCP_WEB_SEARCH_LIMIT`. |

Output is structured content. `title` and `snippet` are left out when the engine
has none, and `results` is empty when nothing matched:

```json
{
  "results": [
    {
      "title": "A title",
      "url": "https://one.example/a",
      "snippet": "A snippet"
    }
  ]
}
```

Results on refused domains are dropped before `limit` is applied.

Errors: `web_search is not configured`, `invalid MCP_WEB_SEARCH_URL`,
`search engine responded with <status>`, `did not return SearXNG style JSON`.
