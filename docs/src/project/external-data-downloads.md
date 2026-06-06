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

Verification was run on 2026-06-06 against Onshape API v14 using the public
CRANK Part Studio from Onshape's translation guide:

```text
did:  e60c4803eaf2ac8be492c18e
wid:  d2558da712764516cc9fec62
eid:  6bed6b43463f6a46a37b4a22
```

The exports used `storeInDocument=false`, so completed translations returned
external data IDs instead of blob element IDs.

| Format | Export endpoint | Download result | Content-Type | Body bytes |
| ------ | --------------- | --------------- | ------------ | ---------- |
| STEP | `createPartStudioTranslation` with `formatName=STEP` | `200`, direct response | `application/octet-stream;charset=utf-8` | 2053444 |
| STL | `createPartStudioTranslation` with `formatName=STL` | `200`, direct response | `application/octet-stream;charset=utf-8` | 3861140 |
| glTF | `createPartStudioExportGltf` with `storeInDocument=false` | `200`, direct response | `application/zip;charset=utf-8` | 18881470 |

3MF was attempted through `createPartStudioTranslation` with `formatName=3MF`.
One request without detail parameters failed with `Invalid 3MF detail parameters
were specified`; a retry with explicit tessellation parameters also failed. This
did not produce an external-data artifact to download, so 3MF-specific export
parameters need separate investigation before comparing its artifact headers.

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

## Client Guidance

- Build the download request from the source document ID and
  `resultExternalDataIds`.
- Send normal Onshape authentication on `downloadExternalData` requests.
- Prefer `Accept: application/octet-stream` for export artifact downloads, even
  though `application/json` was tolerated in the verified case.
- Treat `Content-Length` as optional; rely on the actual bytes read when
  buffering or streaming.
- Preserve `ETag` when caching artifacts and use `If-None-Match` for validation.
- No absolute-URL or redirect-specific helper is needed for the verified
  external-data path. Revisit this only if another external-data host or redirect
  appears in future verification.
