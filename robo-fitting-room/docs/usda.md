# USDA layout

What `compose` writes to the inlined `.usda` inside the output zip,
prim by prim. Use this when you're inspecting the stage in Isaac's
Stage tree, or writing a downstream consumer that reads the USD
directly.

## Top-level prim hierarchy

```
/World                              ← def Xform, defaultPrim
├── PhysicsScene                    ← gravity, GPU broadphase, 120 Hz step
├── robots                          ← def Scope
│   └── <robot_id>                  ← PhysicsArticulationRootAPI + PhysxArticulationAPI
│       ├── world_link              ← URDF root, FixedJoint-anchored
│       ├── like_kr240r2900_*       ← every link: PhysicsRigidBodyAPI + PhysicsMassAPI + PhysxContactReportAPI + IsaacContactSensor child
│       ├── joint_a1 .. joint_a6    ← PhysicsRevoluteJoint with PhysicsDriveAPI:angular
│       └── world_anchor            ← PhysicsFixedJoint pinning the root link
├── fixtures                        ← def Scope
│   └── <fixture_id>
│       └── … static rigid bodies + collision hulls + world_anchor
├── Looks                           ← def Scope of UsdPreviewSurface materials
├── ActionGraph_clock               ← OmniGraph: /<cell>/clock publisher
└── ActionGraph_<safe_robot_id>     ← OmniGraph per robot (motion + contacts)

/Environment                        ← DistantLight default
/Render                             ← OmniverseGlobalRenderSettings + LdrColor
/FlatGrid                           ← Isaac 5.1 default-environment grid
```

<!-- SCREENSHOT-NEEDED: Isaac Sim Stage tree with the demo cell USDA loaded — robots Scope expanded showing one robot's full hierarchy (links + joints + world_anchor + IsaacContactSensor child prims), Render and Environment nodes visible. Save under docs/img/stage-hierarchy.png -->

## Per-robot articulation

- `PhysicsArticulationRootAPI` + `PhysxArticulationAPI` on the robot Xform
- `solverPositionIterationCount = 64`, `solverVelocityIterationCount = 4`
- Self-collisions disabled (`physxArticulation:enabledSelfCollisions = 0`)
- Sleep + stabilisation thresholds tuned for heavy industrial-arm articulations
- Each link uses a single `matrix4d xformOp:transform` carrying its
  forward-kinematics bind pose at the configured **home pose**
  (`[0°, -90°, +90°, 0°, 0°, 0°]` by default; per-robot override via `home_pose_deg`)
- The same home angles are written into each joint's
  `drive:angular:physics:targetPosition`, so the arm sits stowed at scene open

## Per-link mass + inertia

URDFs from many vendors ship with placeholder inertials (e.g. `mass = 2 kg`,
`diag = (0.01, 0.01, 0.01)`) which would explode under stiff drives.
Compose overrides them with steel-arm reference values when the link
name matches a known suffix:

| Link suffix | Mass (kg) | Diagonal inertia (kg·m²) |
|---|---:|---|
| `_base_link` | 250 | (15, 15, 10) |
| `_link_1` | 200 | (10, 10, 8) |
| `_link_2` | 150 | (12, 12, 4) |
| `_link_3` | 120 | (8, 8, 3) |
| `_link_4` | 60 | (3, 3, 1.5) |
| `_link_5` | 30 | (1, 1, 0.5) |
| `_link_6` (with tool flange) | 35 | (0.5, 0.5, 0.4) |

URDF inertials are used verbatim when a link doesn't match the table.
Orphan rigid bodies (URDF root / fixture root with no `<inertial>`)
get a 10 kg / identity-inertia fallback so PhysX never falls back to
its "negative mass small sphere" path. Generalising the override
table beyond KUKA KR240 is tracked as BL-13.

## Per-joint drive

| Attribute | Value |
|---|---|
| `drive:angular:physics:stiffness` | `1.5e5` |
| `drive:angular:physics:damping` | `100` |
| `drive:angular:physics:maxForce` | from URDF `<limit effort=…>` |
| `drive:angular:physics:targetPosition` | matching home-pose angle (degrees) |

Revolute limits emit in degrees, prismatic in metres (PhysX/USD
convention).

## PhysicsScene tuning

```usda
def PhysicsScene "PhysicsScene" (
    prepend apiSchemas = ["PhysxSchemaPhysxSceneAPI", "PhysxSceneAPI"]
)
{
    vector3f basic:gravityDirection = (0, 0, -1)
    float    basic:gravityMagnitude = 9.81
    bool     physxScene:enableGPUDynamics = 1
    bool     physxScene:enableGpuSdfApproximation = 1
    int      physxScene:gpuFoundLostPairsCapacity = 8192
    int      physxScene:gpuFoundLostAggregatePairsCapacity = 8192
    int      physxScene:gpuTotalAggregatePairsCapacity = 4096
    int      physxScene:gpuMaxRigidContactCount = 1048576
    int      physxScene:gpuMaxRigidPatchCount = 16384
    uniform uint physxScene:maxPositionIterationCount = 16
    uniform uint physxScene:maxVelocityIterationCount = 4
    int      physxScene:positionIterations = 16
    int      physxScene:velocityIterations = 4
    float    physxScene:timeStepsPerSecond = 120
}
```

The GPU broadphase capacities are sized for the 8-robot demo cell
at home pose. Scenes with more robots, denser fixtures, or aggressive
contacts may need higher caps; PhysX prints `needs to increase
PxGpuDynamicsMemoryConfig::*` warnings when it spills, with the exact
value to configure.

## Per-link contact instrumentation

Every collision-bearing link carries:

- `PhysxContactReportAPI` on the rigid body (so PhysX surfaces contact events)
- A child `IsaacContactSensor` prim at `<link>/ContactSensor_node` with `enabled = 1`
- An `IsaacReadContactSensor` OmniGraph node referencing that sensor
- A generic `ROS2Publisher` emitting `std_msgs/Bool` on
  `/<cell>/<robot>/contacts/<link>`. The publisher's exec input is wired
  to the sensor's `outputs:execOut`, not `OnPlaybackTick.outputs:tick`,
  so the topic only fires when a contact event lands

## ActionGraphs (ROS 2 wiring)

Each robot gets a `def OmniGraph "ActionGraph_<safe_robot_id>"`
containing:

1. `omni.graph.action.OnPlaybackTick` — drives the per-frame chain
2. `isaacsim.core.nodes.IsaacReadSimulationTime` — sim-time stamp source
3. `isaacsim.ros2.bridge.ROS2PublishJointState` →
   `/<cell>/<robot>/joint_states`
4. `isaacsim.ros2.bridge.ROS2SubscribeJointState` ←
   `/<cell>/<robot>/joint_command`
5. `isaacsim.core.nodes.IsaacArticulationController` — applies the
   subscribed targets
6. `isaacsim.sensors.physics.IsaacReadContactSensor` ×N + matching
   `isaacsim.ros2.bridge.ROS2Publisher` (`std_msgs/Bool`) — one pair
   per collision-bearing link

The scene-level `def OmniGraph "ActionGraph_clock"` publishes
`rosgraph_msgs/Clock` on `/<cell>/clock`.

### Joint-name mapping

`inputs:jointNames` is auto-derived from the URDF actuated-joint
declaration order, with `fixed` joints filtered out — typically
`["joint_a1", "joint_a2", "joint_a3", "joint_a4", "joint_a5", "joint_a6"]`
for a 6-axis arm.

### What is NOT in the graph yet

- A `FollowJointTrajectory` action server (BL-11) — needed before
  MoveIt-style trajectory execution can target the simulator
- Cell-level collision aggregation (BL-24) — a single
  `/<cell>/contacts/v1` topic with rich event data (force, normal,
  the other body). Pure-OmniGraph aggregation was attempted and
  reverted; the path forward is a sidecar pod that subscribes to the
  per-link Bool feeds and republishes a typed event stream

## Determinism

Identical manifests produce byte-identical USDA output. Two
consecutive `compose` runs of the same manifest yield identical zip
contents (NFR-105). The output is also independent of build
configuration: a `cargo build --release` and a `cargo build` produce
the same USDA from the same manifest.

This matters for CI / regression testing. The `tests/openapi_snapshot`
+ `tests/real_assets` tests rely on the determinism
guarantee.
