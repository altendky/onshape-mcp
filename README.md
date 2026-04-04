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

Access to the hosted servers is currently limited to a closed testing group.
Don't call us, we'll call you.

## Getting Started

### Local Setup (Claude Desktop, OpenCode, or any MCP client)

This is the recommended path. A local server can read and write files on your
machine directly — screenshots, exports, and FeatureScript — instead of passing
content through the conversation.

**Step 1: Get the server binary**

Pick whichever is easier for you:

- **npx** (requires [Node.js](https://nodejs.org/)): No separate install
  needed — your MCP client runs `npx --yes onshape-mcp` and it downloads
  automatically.
- **Pre-built binary**: Download from
  [GitHub Releases](https://github.com/altendky/onshape-mcp/releases) and save
  it somewhere convenient.

**Step 2: Configure your MCP client**

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

**Step 3: Try it**

Ask your AI assistant to create a new Onshape document with a simple shape. If
you haven't authenticated yet, it will give you a URL to open in your browser to
log into Onshape and approve access.

### Web Setup (Claude.ai or other web MCP clients)

No local installation needed. Add `https://onshape.mcp.fstab.net/mcp` as a
remote MCP server in your client's settings. You'll be prompted to log into
Onshape when you first use it.

The web transport has no access to your local filesystem. FeatureScript and
other file content passes through the conversation instead of being read from
and written to disk, which uses significantly more tokens.

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
