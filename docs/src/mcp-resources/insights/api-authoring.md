# Document and Configuration Authoring

Use these rules when constructing request bodies for `createDocument` and
`updateConfiguration` through `onshape_api_call`.

## Create a Document

Pass the MCP `body` argument directly as this object shape:

```json
{"name":"<nonempty name>","description":"<description>","parentId":"<opaque folder ID>","isPublic":true}
```

Legacy callers may pass a serialized JSON string for compatibility. Do not
JSON-encode that resulting string again.

- `name` is semantically required and must not be blank, although
  `BTDocumentParams` does not mark it OpenAPI-required.
- `parentId` is the destination folder's opaque ID, not a folder name or path.
  Spaces in the folder name are irrelevant because the name is not sent.
- Free accounts require `isPublic: true`.
- Omit `ownerId`, `ownerEmail`, `ownerType`, and `projectId` unless deliberately
  assigning ownership or project metadata supported by the account and API.
- Omit debugging/internal fields and optional fields whose value would be
  `null`, unless a deliberate API behavior specifically requires them.
- The MCP server does not apply defaults declared in the OpenAPI schema. Send
  every value on which the request depends.

`createDocument` is locally checked for an object body, a non-blank string
`name`, and the documented top-level field types. Unknown
fields and deliberate `null` values for optional fields are left for Onshape to
interpret so the generic tool does not block valid API extensions or explicit
optional behavior.

## Author Configuration Nodes

`nodeId` is an opaque, server-owned `BTObjectId`. It is serialized object
identity, not a value derived from `parameterName`, `parameterId`, an option
name, a UUID, or a guessed regular expression.

When adding new nodes to `configurationParameters`, their options, or
`currentConfiguration`:

- Omit `nodeId` on newly authored nodes and entries.
- Submit the configuration update, then call `getConfiguration` again.
- Preserve every exact `nodeId` returned by Onshape when sending later updates
  to those existing nodes.
- Never synthesize, normalize, parse, or replace a returned `nodeId`.

Use a read-modify-write workflow for existing configurations: read the current
configuration, retain its server-owned identity and serialization fields, make
only the intended changes, update it, and read it back to verify the resulting
IDs and values.

The generic `onshape_api_call` tool does not reject or warn about supplied
configuration `nodeId` values. It is stateless and cannot reliably distinguish a
new caller-authored node from an existing node carrying a valid ID returned by
Onshape. A warning based only on presence or ID format would produce false
positives for valid updates, so this safeguard remains explicit authoring
guidance rather than unreliable runtime inference.
