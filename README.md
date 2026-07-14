# Onshape MCP

AI-assisted access to your Onshape CAD documents.

## What This Is

Onshape MCP connects your AI assistant to Onshape. You describe what you want
in Onshape terms — create a sketch, extrude a part, export an STL — and the AI
handles the API calls. This is a helper for people who already know Onshape,
not a replacement for CAD skills.

## Status

Early development. This is offered for people to try if they're interested —
things may not always work as expected, and feedback is appreciated.

This project does not provide a public hosted MCP server or a public OAuth
proxy. Run it locally, self-host it, or connect to a server operated by someone
you independently trust.

## Getting Started

### Local Setup (Claude Desktop, OpenCode, or any MCP client)

This is the recommended path. A local server can read and write files on your
machine directly — screenshots, exports, and FeatureScript — instead of passing
content through the conversation.

**Step 1: Create an Onshape OAuth application**

Create your own OAuth application in Onshape with
`http://localhost:18338/callback` as a redirect URL. Keep its client ID and
client secret available for `onshape-mcp auth login`. Direct OAuth with these
user-owned credentials is the default. API access and secret keys are also
supported as an alternative; see
[Authentication](docs/src/project/authentication.md).

**Step 2: Get the server binary**

Pick whichever is easier for you:

- **npx** (requires [Node.js](https://nodejs.org/)): No separate install
  needed — your MCP client runs `npx --yes onshape-mcp` and it downloads
  automatically.
- **Pre-built binary**: Download from
  [GitHub Releases](https://github.com/altendky/onshape-mcp/releases) and save
  it somewhere convenient.

**Step 3: Configure your MCP client**

Tell your MCP client how to launch the server.

#### Claude Desktop

Add to your Claude Desktop config (`claude_desktop_config.json`):

Using npx:

```json
{
  "mcpServers": {
    "onshape": {
      "command": "npx",
      "args": ["--yes", "onshape-mcp"]
    }
  }
}
```

Using the downloaded binary:

```json
{
  "mcpServers": {
    "onshape": {
      "command": "/path/to/onshape-mcp"
    }
  }
}
```

#### OpenCode

Add to your `opencode.json`:

Using npx:

```json
{
  "mcp": {
    "onshape": {
      "type": "local",
      "command": ["npx", "--yes", "onshape-mcp"]
    }
  }
}
```

Using the downloaded binary:

```json
{
  "mcp": {
    "onshape": {
      "type": "local",
      "command": ["/path/to/onshape-mcp"]
    }
  }
}
```

#### Other MCP clients

Any MCP client that supports stdio transport can launch this server. The command
is either `npx --yes onshape-mcp` or the path to the downloaded binary.

**Step 4: Authenticate and try it**

Configure your OAuth client ID and secret, then run
`npx --yes onshape-mcp auth login` when using the npm package, or
`onshape-mcp auth login` when using the downloaded binary.
Prefer configuration or `ONSHAPE_MCP_AUTH__CLIENT_ID` and
`ONSHAPE_MCP_AUTH__CLIENT_SECRET` environment variables. The supported
`--client-id` and `--client-secret` flags may expose secrets in shell history or
process listings. Restart or start your MCP client, then ask your AI assistant
to create a new Onshape document with a simple shape.

### Experimental Remote Setup

Streamable HTTP remains available for self-hosting, but it is experimental and
has not been broadly verified. This project offers no publicly operated
endpoint. If an independent operator gives you access, add a placeholder URL such as
`https://mcp.example.com/mcp` to your web MCP client and confirm the operator's
trust, access, and retention policies first. ChatGPT connectivity is currently
known to fail; see [#546](https://github.com/altendky/onshape-mcp/issues/546).

HTTP transport deliberately disables MCP file reads and writes. File-reference
arguments and local output paths therefore cannot access your computer; provide
supported request content inline and retrieve remote results through the tool
response instead.

## Tips

- **Share document URLs.** Paste an Onshape document URL into the conversation
  to give the AI context about what you're working with.
- **Use Onshape vocabulary.** Say "extrude," "fillet," "sketch on the top
  face," etc. The AI has built-in knowledge about Onshape operations.
- **Multiple steps are normal.** The AI discovers API endpoints dynamically, so
  a single request may involve several steps behind the scenes.
- **Be specific when things go wrong.** If the AI takes a wrong turn, try
  describing the Onshape operation more precisely.

## What You Can Do

- Browse and explore your Onshape documents
- Create new documents and parts
- Build features: sketches, extrudes, revolves, sweeps, fillets, construction
  planes
- Take screenshots of Part Studios from different angles
- Export parts (STL, STEP, and other formats)
- Work with FeatureScript — write custom features and debug existing ones

The AI has access to the full Onshape REST API, so anything available through
the API is potentially reachable. Results will vary — some operations work more
reliably than others at this stage.

## Supported Platforms

| Platform | Architecture |
| -------- | ------------ |
| Linux | x86_64, aarch64 |
| macOS | x86_64, aarch64 (Apple Silicon) |
| Windows | x86_64 |

## Project Documentation

For development, architecture, and contribution details, see
[`docs/src/project/`](docs/src/project/index.md).

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or
  <http://opensource.org/licenses/MIT>)

at your option.
