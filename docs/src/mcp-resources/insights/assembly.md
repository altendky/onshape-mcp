# Assembly

Build and edit assemblies with as few Onshape API calls as possible. Every
successful call counts against the account's annual API limit, including a
write whose feature regenerates with an error.

## Tools (one API call each)

| Task | Tool |
| --- | --- |
| Instances, occurrence paths, fixed state, positions, mates and what they join | `onshape_assembly_read` |
| Mate between two existing connectors | `onshape_mate` |
| Screw, gear, rack-and-pinion or linear relation | `onshape_mate_relation` |
| Assembly connector on a hole edge, cylinder axis, face, vertex, ... | `onshape_mate_connector` |

The write tools return the new feature ID and its regeneration status from
the same response. A status other than `OK` means the call still counted: find
the cause, then delete or fix the feature before building on it.

## Choosing connectors

1. **Part Studio connectors owned by the part.** They travel into every
   assembly. `onshape_assembly_read` lists them under `part_studio_connectors`;
   reference them in `onshape_mate` with `kind: "part_studio"`, the instance's
   occurrence path and the connector's feature ID. One call per mate.
2. **Assembly connectors placed by inference** with `onshape_mate_connector`:
   `CENTER` of a hole's circular edge, `TOP/MID/BOTTOM_AXIS_POINT` of a
   cylinder, `CENTROID` of a planar face. Then mate them. Two to three calls
   per mate.
3. Offsets only when no geometry gives the frame. Never offset a face-centroid
   connector onto a hole axis: it depends on the face's inferred X axis and
   breaks on patterned faces.

## Frame rules

- A mate makes both connector frames coincide in the same direction. Two
  faces in contact have opposite normals, so use `flip: true` or choose
  same-direction faces.
- Revolute, slider and cylindrical mates use only the Z axis. Fastened and
  planar mates also use X, which sets the clocking.
- On a planar face, an inferred connector's X axis is the assembly X axis
  projected onto the face (observed with instances at identity).
- An `OK` status does not prove the pose. A cylindrical mate can be satisfied
  by turning the part 180 degrees about the axis. Check the occurrence's
  rotation with `onshape_assembly_read` after the first mate of a new pattern.
- Redundant mates (two parallel cylindricals on one part) must agree to about
  1e-8 m, or every mate in the loop fails. Connectors taken from real hole
  geometry agree; hand-typed offsets usually do not.
- Instances inserted through the API are not fixed. Fix the base part first,
  or the solver may move it.

## Recipes for `onshape_api_call`

From the Onshape API guide for assemblies.

**Delete, suppress and move instances** (`modify`, POST
`/assemblies/d/{did}/w/{wid}/e/{eid}/modify`):

```json
{"deleteInstances": ["<instanceId>"]}
{"suppressionStates": {"<instanceId>": {"value": "true"}}}
{"transformDefinitions": [{"isRelative": false,
  "occurrences": [{"path": ["<topInstance>", "<childInstance>"]}],
  "transform": [1,0,0,0, 0,1,0,0, 0,0,1,1.534, 0,0,0,1]}]}
```

Transforms are 4x4 row-major matrices in metres. Occurrence transforms are
absolute in the top-level assembly's frame.

**Read one feature's full JSON** (`getFeatures` with the `featureId` query
parameter). Use it to copy the exact payload of a mate made in the UI before
writing a similar one.

## Not yet verified against live Onshape

- `flip` on `onshape_mate` writes `primaryAxisAlignment`; `flip` on
  `onshape_mate_connector` writes `flipPrimary`. Both names come from other
  open-source Onshape clients.
- Implicit connectors stored inside a single mate (as the UI does) would make
  geometry mates one call. The `BTMMate-64` schema has a `mateConnectors`
  list for this, but no tested payload exists yet. Capture one from a UI mate
  with `getFeatures` before adding it.
