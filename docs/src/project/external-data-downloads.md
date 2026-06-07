# External Data Downloads

This page records observed behavior for Onshape external-data downloads used by
asynchronous translation/export flows.

## Endpoint

Use `downloadExternalData` after an async export reaches `requestState=DONE`:

```text
GET /documents/d/{did}/externaldata/{fid}
```

The `{fid}` value is one of the completed translation response's
`resultExternalDataIds`. The document ID is the source document ID from the
translation context.

## Verified Behavior

Verification was run on 2026-06-06 and 2026-06-07 against Onshape API v14 using
the public CRANK document from Onshape's translation guide:

```text
did:  e60c4803eaf2ac8be492c18e
wid:  d2558da712764516cc9fec62
Part Studio eid:  6bed6b43463f6a46a37b4a22
Assembly eid:     23a9385cd48c50167c32d6d1
```

The exports used `storeInDocument=false`, so completed translations returned
external data IDs instead of blob element IDs.

| Format | Export endpoint | Download result | Content-Type | Body bytes |
| ------ | --------------- | --------------- | ------------ | ---------- |
| STEP | `createPartStudioTranslation` with `formatName=STEP` | `200`, direct response | `application/octet-stream;charset=utf-8` | 2053444 |
| STL | `createPartStudioTranslation` with `formatName=STL` | `200`, direct response | `application/octet-stream;charset=utf-8` | 3861140 |
| glTF | `createPartStudioExportGltf` with `storeInDocument=false` | `200`, direct response | `application/zip;charset=utf-8` | 18881470 |
| 3MF | `createPartStudioTranslation` with coarse mesh detail parameters | `200`, direct response | binary wrapper did not expose headers | body starts with ZIP `PK` signature |
| 3MF | `translateFormat` for Assembly with coarse mesh detail parameters | `200`, direct response | binary wrapper did not expose headers | body starts with ZIP `PK` signature |

`getAllTranslatorFormats` reports `3MF` as `validDestinationFormat=true`,
`couldBeAssembly=true`, and `contentType=application/3mf`. Minimal 3MF requests
failed quickly through `createPartStudioTranslation` with `Invalid 3MF detail
parameters were specified`, including a variant that only added `translate=true`.
Fine-detail mesh requests were accepted but later failed: the Part Studio result
reported a generic `failed to translate`, and the Assembly result reported
`Memory limit exceeded. Reducing file size may allow translation to succeed`.

The working 3MF requests used coarse flat tessellation fields on the generic
translation endpoints:

```json
{
  "formatName": "3MF",
  "storeInDocument": false,
  "destinationName": "crank-3mf-coarse",
  "notifyUser": false,
  "angularTolerance": 0.5,
  "distanceTolerance": 0.01,
  "maximumChordLength": 0.1,
  "resolution": "coarse",
  "unit": "MILLIMETER"
}
```

The Part Studio translation completed with `resultExternalDataIds` containing
`6a2575236577e5fca6796d82`. The Assembly translation completed with
`resultExternalDataIds` containing `6a2575246577e5fca6796d84`. Downloading each
external-data ID returned a binary artifact with a ZIP signature and an internal
`3D/3dmodel.model` entry, consistent with a 3MF package. The MCP API wrapper used
for the live check returned the binary body but did not expose response headers,
so exact 3MF `Content-Type`, `Content-Disposition`, `ETag`, and filename behavior
still need raw HTTP header capture before being compared to STEP, STL, and glTF.

## Accept Header

For the verified STEP artifact, Onshape returned the same `200` artifact
response for all tested `Accept` values:

| Accept | Result |
| ------ | ------ |
| `application/octet-stream` | `200`, artifact body |
| `application/json` | `200`, artifact body |
| `*/*` | `200`, artifact body |

`application/octet-stream` is still the clearest caller intent for artifact
downloads and should be preferred by export-oriented callers. The current common
client default of `Accept: application/json` is tolerated by this endpoint for
the verified artifact, so no immediate code change is required for
`downloadExternalData` solely because of `Accept` negotiation.

## Redirects And Authentication

External-data artifact downloads returned direct `200` responses in the verified
cases. No `3xx` response or `Location` header was observed when redirects were
disabled with `curl --max-redirs 0`.

Unauthenticated access to the generated external-data URL returned `401` with a
JSON error response. Authenticated access with the normal Onshape bearer token
returned the artifact. Because no redirect was observed, there was no separate
redirect target requiring special authentication handling.

This differs from Onshape synchronous export endpoints, which Onshape documents
as returning `307` redirects that callers must follow with authentication.

## Response Headers

Completed artifact responses included these useful headers:

| Header | Observed behavior |
| ------ | ----------------- |
| `Content-Type` | Artifact media type, commonly `application/octet-stream;charset=utf-8`; glTF export returned `application/zip;charset=utf-8`. |
| `Content-Disposition` | Present as `attachment; filename*=UTF-8''...` using the requested destination name and file extension; `filename*` is the RFC 5987-encoded parameter within this header value. |
| `ETag` | Present on completed artifact responses. |
| `Content-Length` | Not present on the verified HTTP/2 `200` artifact responses. |

`If-None-Match` with the returned STEP `ETag` produced `304` with
`Content-Length: 0` and no response body.

The successful 3MF artifacts were downloaded and verified as binary 3MF package
bodies, but exact response headers were not captured during the 2026-06-07 live
check because the available API wrapper did not return headers for binary
responses.

## Client Guidance

- Build the download request from the source document ID and
  `resultExternalDataIds`.
- Send normal Onshape authentication on `downloadExternalData` requests.
- Prefer `Accept: application/octet-stream` for export artifact downloads, even
  though `application/json` was tolerated in the verified case.
- Treat `Content-Length` as optional; rely on the actual bytes read when
  buffering or streaming.
- Preserve `ETag` when caching artifacts and use `If-None-Match` for validation.
- For 3MF, provide flat mesh detail fields on the generic translation request;
  minimal `formatName=3MF` requests can fail validation.
- Use coarse tessellation or otherwise control triangle count for 3MF when
  exporting larger Part Studios or Assemblies.
- No absolute-URL or redirect-specific helper is needed for the verified
  external-data path. Revisit this only if another external-data host or redirect
  appears in future verification.
