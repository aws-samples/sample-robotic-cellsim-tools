# Driving the Demo Cell from ROS 2 (Windows + Pixi + Isaac Sim)

This guide walks through testing the per-robot ActionGraphs that
`robo-fitting-room` emits — manually moving joints with `ros2 topic pub`,
reading joint state, and confirming collisions stop the arm.

Assumes:

- Isaac Sim 5.1 Full installed at `C:\isaac-sim`
- The composed scene (`demo-cell.zip`) extracted and loaded
- A Pixi environment on your Windows box that already includes ROS 2 Humble
  (or Jazzy) — referred to below as the **ROS terminal**
- An empty PowerShell where Isaac Sim is launched — the **Isaac terminal**

If `ros2 topic list` already works in your Pixi env you can skip directly to
[§3 — Verify topics](#3-verify-topics).

---

## 1. Pre-flight inside Isaac

1. **Window → Extensions** → search **`isaacsim.ros2.bridge`**.
2. Toggle **Enabled** (green dot). If the toggle says "library not found",
   set the bundled DDS path before launch (next step).
3. Quit Isaac.
4. Edit (or create) the launcher batch file you use to start Isaac and add
   the bundled FastDDS profile **before** the kit executable line. Example
   `isaac-sim.bat`:

   ```bat
   @echo off
   set RMW_IMPLEMENTATION=rmw_fastrtps_cpp
   set ROS_DOMAIN_ID=0
   set FASTRTPS_DEFAULT_PROFILES_FILE=C:\isaac-sim\extscache\isaacsim.ros2.bridge-*\humble\fastdds.xml
   call "C:\isaac-sim\isaac-sim.bat"
   ```

   Setting `ROS_DOMAIN_ID=0` matters: the ROS terminal must use the **same**
   domain or it won't see Isaac's topics.

5. Relaunch Isaac via that script. In the console you should see one
   `[isaacsim.ros2.bridge] Successfully initialized` line during startup.

## 2. Pre-flight in the ROS terminal (Pixi)

```powershell
# In a fresh shell — NOT inside Isaac:
cd C:\path\to\your\pixi\workspace
pixi shell
set ROS_DOMAIN_ID=0
ros2 doctor --report
```

If `ros2` is not on PATH inside the Pixi shell, your env is missing the
`ros-humble-desktop` (or `ros-jazzy-desktop`) feature. Add it to
`pixi.toml`, run `pixi install`, then re-enter the shell.

## 3. Verify topics

Open the composed scene, press **Play** (spacebar), then in the ROS
terminal:

```powershell
ros2 topic list
```

Expected — one pair per robot in the manifest plus the scene clock:

```
/clock
/robot_01/joint_command
/robot_01/joint_states
/robot_02/joint_command
/robot_02/joint_states
/robot_03/joint_command
/robot_03/joint_states
/robot_04/joint_command
/robot_04/joint_states
/robot_05/joint_command
/robot_05/joint_states
/robot_06/joint_command
/robot_06/joint_states
/robot_07/joint_command
/robot_07/joint_states
/robot_08/joint_command
/robot_08/joint_states
```

Topic names follow the `robot_id` exactly as authored in the YAML manifest.

## 4. Read live joint state

```powershell
ros2 topic echo /robot_01/joint_states --once
```

You should see a `sensor_msgs/JointState` message with **6 names**
(`joint_a1`..`joint_a6`) and matching `position`, `velocity`, `effort`
arrays. Positions are in **radians** (the bridge respects SI even when the
property panel shows degrees).

If nothing comes back, jump to [§7 — Troubleshooting](#7-troubleshooting).

## 5. Send a joint command

```powershell
ros2 topic pub --once /robot_01/joint_command sensor_msgs/msg/JointState `
  "{name: ['joint_a1','joint_a2','joint_a3','joint_a4','joint_a5','joint_a6'], `
    position: [0.5, -1.4, 1.4, 0.0, 0.7, 0.0]}"
```

The arm should track to that pose. Defaults apply at scene open
(`a1=0, a2=-90°, a3=+90°, …`), so the message above bends `a1`/`a5` while
keeping the home elbow.

**Important:**

- Positions are **radians** for revolute joints, **metres** for prismatic.
- The `name` array must match the **URDF joint names exactly**. They are the
  6 actuated joints (`joint_a1`..`joint_a6`); fixed joints (e.g.
  `tool_attach`, `world_anchor`) are not commanded.
- Sending `velocity` or `effort` mid-stream conflicts with the position
  drive — drop them unless you specifically know you want torque control.

## 6. Drive in a loop (collision test)

This script swings `joint_a1` slowly while keeping the elbow folded; aim
the swing so the arm physically meets a fixture. With the steel-density
mass + convex-hull collisions in place, the drive will **stall** instead of
penetrating the fixture, and the joint position will plateau before the
commanded value. That's how you confirm collision response is working.

```powershell
python -c @"
import math, time, rclpy
from rclpy.node import Node
from sensor_msgs.msg import JointState

rclpy.init()
n = rclpy.create_node('arm_demo')
pub = n.create_publisher(JointState, '/robot_08/joint_command', 10)
sub = []
def on_state(msg): sub.append(msg)
n.create_subscription(JointState, '/robot_08/joint_states', on_state, 10)
joints = ['joint_a1','joint_a2','joint_a3','joint_a4','joint_a5','joint_a6']
t0 = time.time()
while rclpy.ok() and time.time() - t0 < 30:
    msg = JointState()
    msg.name = joints
    msg.position = [1.5*math.sin(time.time()-t0), -1.4, 1.4, 0.0, 0.7, 0.0]
    pub.publish(msg)
    rclpy.spin_once(n, timeout_sec=0.05)
if sub:
    last = sub[-1]
    print('last position[a1] =', last.position[0])
"@
```

Run that, watch the viewport: `joint_a1` should swing freely until the
arm meets a fixture/another robot, then the position should hold short
of the commanded angle even though the publish keeps repeating.

## 7. Troubleshooting

| Symptom | Likely cause | Fix |
|---|---|---|
| `ros2 topic list` empty during Play | Bridge ext disabled, or `ROS_DOMAIN_ID` mismatch | Re-check §1 step 4; both terminals must show the same `ROS_DOMAIN_ID` |
| Topics visible but `echo` blank | OmniGraph isn't ticking | In Isaac, **Window → Visual Scripting → Action Graph**, open `/World/ActionGraph_robot_01`, watch the green pulse on the `OnPlaybackTick → PublishJointState` link |
| `pub` succeeds but robot ignores it | `name[]` mismatch | Verify each name with `ros2 topic echo /<robot>/joint_states` first; names must match URDF case-for-case |
| Robot twitches/snaps to target | Drive stiffness too high for link mass | We default to `1e7` stiffness / `1e5` damping; if too aggressive, reduce in `compose.rs::emit_joint` (and recompose) |
| `joint_states` missing some joints | Fixed joints aren't published, that's expected | The 6 actuated joints per arm are what the bridge reports |
| Two robots see each other's commands | Topics share a domain | Use distinct `ROS_DOMAIN_ID` per Isaac instance, or rename the robots in the manifest |
| Wall-clock vs sim-clock mismatch in MoveIt/rviz | Subscriber not using sim time | `ros2 run rviz2 rviz2 --ros-args -p use_sim_time:=true` |

## 8. Useful one-liners

```powershell
# What's the rate of /joint_states?
ros2 topic hz /robot_01/joint_states

# Discover the message type the publisher is using
ros2 topic info /robot_01/joint_states --verbose

# Plot a joint's position over time (rqt_plot must be in your Pixi env)
ros2 run rqt_plot rqt_plot /robot_01/joint_states/position[0]

# Echo the simulation clock — confirms the scene-level ActionGraph is alive
ros2 topic echo /clock --once
```

---

When step 4 (`echo /joint_states`) returns data but step 5 doesn't move
the robot, paste the Isaac console output (filter on `ros2.bridge` and
`omni.graph`) and we'll narrow it down.
