use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use urdf_robo_dresser::{parse_urdf, JointType};

use crate::error::R3Error;

/// Per-axis affine correction applied before clamping:
/// `rad = sign * deg * (pi/180) + offset_rad`. Default is the identity
/// (plain degrees→radians), so an un-tuned run is a faithful unit conversion.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AxisMap {
    #[serde(default = "default_sign")]
    pub sign: f64,
    #[serde(default)]
    pub offset_rad: f64,
}

fn default_sign() -> f64 {
    1.0
}

impl Default for AxisMap {
    fn default() -> Self {
        Self {
            sign: 1.0,
            offset_rad: 0.0,
        }
    }
}

fn default_axes() -> [AxisMap; 6] {
    [AxisMap::default(); 6]
}

fn default_warn() -> bool {
    true
}

/// How captured degrees become commanded radians. The captured KUKA axis
/// values can sit outside the URDF range (different zero/sign convention), so
/// the affine `default_axes`/`per_robot` knobs exist to reconcile them; the
/// optional clamp keeps a mis-tuned run inside the joint limits (loudly).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MappingConfig {
    #[serde(default)]
    pub clamp_to_limits: bool,
    #[serde(default = "default_warn")]
    pub warn_out_of_range: bool,
    #[serde(default = "default_axes")]
    pub default_axes: [AxisMap; 6],
    /// robot_id -> six-axis override of `default_axes`.
    #[serde(default)]
    pub per_robot: BTreeMap<String, [AxisMap; 6]>,
}

impl Default for MappingConfig {
    fn default() -> Self {
        Self {
            clamp_to_limits: false,
            warn_out_of_range: true,
            default_axes: default_axes(),
            per_robot: BTreeMap::new(),
        }
    }
}

impl MappingConfig {
    /// Resolve the six AxisMaps for a robot: its per-robot override if any,
    /// else the cell-wide defaults.
    pub fn axes_for(&self, robot_id: &str) -> [AxisMap; 6] {
        self.per_robot
            .get(robot_id)
            .copied()
            .unwrap_or(self.default_axes)
    }
}

/// The six actuated joint names + their radian limits, in URDF declaration
/// order (joint_a1..joint_a6 for the KR-class robots).
#[derive(Debug, Clone, PartialEq)]
pub struct JointLimits {
    pub names: [String; 6],
    pub lower: [f64; 6],
    pub upper: [f64; 6],
}

// FR-200. Extract the actuated (non-Fixed) joints + radian limits from a
// dressed URDF, reusing the dresser's parser. Errors if the URDF doesn't have
// exactly six actuated joints (the capture is six-axis).
pub fn joint_limits_for_robot(dressed_urdf: &Path) -> Result<JointLimits, R3Error> {
    let robot = parse_urdf(dressed_urdf)
        .map_err(|e| R3Error::Urdf(format!("{}: {}", dressed_urdf.display(), e)))?;
    let actuated: Vec<_> = robot
        .joints
        .iter()
        .filter(|j| !matches!(j.joint_type, JointType::Fixed))
        .collect();
    if actuated.len() != 6 {
        return Err(R3Error::Urdf(format!(
            "{}: expected 6 actuated joints, found {}",
            dressed_urdf.display(),
            actuated.len()
        )));
    }
    let mut names: [String; 6] = Default::default();
    let mut lower = [f64::NEG_INFINITY; 6];
    let mut upper = [f64::INFINITY; 6];
    for (i, j) in actuated.iter().enumerate() {
        names[i] = j.name.clone();
        if let Some(l) = &j.limit {
            lower[i] = l.lower;
            upper[i] = l.upper;
        }
    }
    Ok(JointLimits {
        names,
        lower,
        upper,
    })
}

// FR-201, FR-202, FR-203. Map six captured degree values to commanded radians:
// affine per axis, then optional clamp to the URDF limit. `None` inputs (no
// sample yet) pass through as `None`. Returns the radians plus the count of
// axes that were clamped this call (for reporting / warnings).
pub fn map_degrees_to_radians(
    deg: &[Option<f64>; 6],
    axes: &[AxisMap; 6],
    limits: &JointLimits,
    clamp: bool,
    warn: bool,
) -> ([Option<f64>; 6], u32) {
    const DEG2RAD: f64 = std::f64::consts::PI / 180.0;
    let mut out: [Option<f64>; 6] = [None; 6];
    let mut clamped = 0u32;
    for i in 0..6 {
        let Some(d) = deg[i] else { continue };
        let raw = axes[i].sign * d * DEG2RAD + axes[i].offset_rad;
        let (lo, hi) = (limits.lower[i], limits.upper[i]);
        let out_of_range = raw < lo || raw > hi;
        let val = if clamp && out_of_range {
            clamped += 1;
            raw.clamp(lo, hi)
        } else {
            if out_of_range && warn {
                log::warn!(
                    "{} target {:.4} rad outside limit [{:.4}, {:.4}] (axis {})",
                    limits.names[i],
                    raw,
                    lo,
                    hi,
                    i + 1
                );
            }
            raw
        };
        out[i] = Some(val);
    }
    (out, clamped)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kr_a2_limits() -> JointLimits {
        // Real joint_a2 bound from the dressed kr240r2900 URDF.
        JointLimits {
            names: [
                "joint_a1".into(),
                "joint_a2".into(),
                "joint_a3".into(),
                "joint_a4".into(),
                "joint_a5".into(),
                "joint_a6".into(),
            ],
            lower: [-3.2288591, -2.7052601, -2.268928, -6.1086524, -2.268928, -6.1086524],
            upper: [3.2288591, 0.6108652, 2.687807, 6.1086524, 2.268928, 6.1086524],
        }
    }

    // FR-202: default identity map is a plain degrees->radians conversion.
    #[test]
    fn identity_is_plain_deg_to_rad() {
        let axes = [AxisMap::default(); 6];
        let deg = [Some(90.0), None, None, None, None, None];
        let (out, clamped) = map_degrees_to_radians(&deg, &axes, &kr_a2_limits(), false, false);
        assert!((out[0].unwrap() - std::f64::consts::FRAC_PI_2).abs() < 1e-9);
        assert_eq!(clamped, 0);
    }

    // FR-201/203: the real -200.57 deg A2 sample is out of range; clamp pins
    // it to the lower limit, no-clamp lets it through unchanged.
    #[test]
    fn out_of_range_a2_clamps_to_limit() {
        let axes = [AxisMap::default(); 6];
        let deg = [None, Some(-200.5738), None, None, None, None];
        let limits = kr_a2_limits();

        let (no_clamp, c0) = map_degrees_to_radians(&deg, &axes, &limits, false, false);
        assert!((no_clamp[1].unwrap() - (-200.5738_f64).to_radians()).abs() < 1e-9);
        assert_eq!(c0, 0);

        let (clamped, c1) = map_degrees_to_radians(&deg, &axes, &limits, true, false);
        assert!((clamped[1].unwrap() - limits.lower[1]).abs() < 1e-9);
        assert_eq!(c1, 1);
    }

    // FR-201: a per-axis offset can pull the sample back into range.
    #[test]
    fn offset_brings_a2_into_range() {
        let mut axes = [AxisMap::default(); 6];
        axes[1].offset_rad = std::f64::consts::PI; // +180 deg
        let deg = [None, Some(-200.5738), None, None, None, None];
        let limits = kr_a2_limits();
        let (out, clamped) = map_degrees_to_radians(&deg, &axes, &limits, true, false);
        let v = out[1].unwrap();
        assert!(v >= limits.lower[1] && v <= limits.upper[1]);
        assert_eq!(clamped, 0);
    }
}
