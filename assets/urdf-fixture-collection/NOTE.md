## Fixture URDF Requirements

**Strict requirement:** Each fixture URDF defines a static (non-articulated) piece of cell infrastructure positioned at its own local origin `(0, 0, 0)`.

Fixtures are placed into scenes via the scene YAML manifest. The manifest specifies the world-space position and orientation for each fixture instance.

### What qualifies as a fixture

- Joining fixtures (welding/riveting stations)
- Conveyors and shuttles
- Safety fences and barriers
- Part racks and storage
- Any static geometry that robots interact with but does not move independently

### Rules

1. The coordinate origin `(0, 0, 0)` is the fixture's placement reference point (typically the base center or mounting point)
2. All geometry (visual and collision meshes) is positioned relative to this reference point
3. Fixtures MUST use only `fixed` joints — no actuated joints allowed
4. Each link SHOULD have both visual and collision geometry defined
5. Named links can serve as target frames for robot reachability checks (e.g., `target_frame_1`)

### Directory Convention

Each fixture lives in its own directory: `<FIXTURE_NAME>/<FIXTURE_NAME>.urdf` with meshes in `<FIXTURE_NAME>/meshes/`.

```
urdf-fixture-collection/
└── JOINING_FIXTURE_FX01/
    ├── JOINING_FIXTURE_FX01.urdf
    └── meshes/
        ├── base.obj
        ├── clamp_left.obj
        └── clamp_right.obj
```

### Note

URDF files and meshes sourced from customer data are not checked in here.
