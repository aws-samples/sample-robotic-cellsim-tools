## Floor Plan Requirements

Floor plans are 2D CAD exports (DXF→OBJ) containing only vertices (`v`) and line elements (`l`). They represent factory cell boundaries, fences, columns, and robot work envelopes as flat line geometry at z=0.

### OBJ Format Convention

- Only `v` (vertex) and `l` (line) elements — no faces
- All geometry at z=0.0 (2D plan view)
- Coordinates in meters, pre-offset to cell-local origin
- Header comments document the source DXF file and original origin offset
- Layer separation via `# Layer: <name>` comments or `o <group_name>` groups

### Naming Convention

`<CELL_NAME>-floor-plan.obj`

### Note

OBJ files sourced from customer CAD data are not checked in here.
