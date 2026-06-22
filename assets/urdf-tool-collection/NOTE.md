## Tool URDF Requirements

**Strict requirement:** The tool URDF origin point `(0, 0, 0)` MUST be located at the exact center of the tool's mounting flange interface.

This is the docking point where the tool connects to the robot's end-effector. The dresser uses this origin as the rotation pivot and attachment reference.

### Rules

1. The coordinate origin `(0, 0, 0)` must coincide with the center of the tool's mounting/docking flange
2. All tool geometry (visual and collision meshes) is positioned relative to this docking center
3. The tool's mounting surface faces should be approximately at the origin plane
4. Tool geometry extends away from the origin into the working direction

### Directory Convention

Each tool lives in its own directory: `<TOOL_NAME>/<TOOL_NAME>.urdf` with meshes in `<TOOL_NAME>/meshes/`.

### Note

URDF files and meshes sourced from customer data are not checked in here.
