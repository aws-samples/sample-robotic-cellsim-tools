//! Small 4x4 homogeneous matrix utilities used by the FK pass in compose.
//! Conventions: row-major matrices, ZYX Euler for RPY (matching URDF semantics).

pub type Mat4 = [[f64; 4]; 4];

pub const IDENTITY: Mat4 = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

pub fn from_translation_rpy(translation: [f64; 3], rpy: [f64; 3]) -> Mat4 {
    let [roll, pitch, yaw] = rpy;
    let (sr, cr) = roll.sin_cos();
    let (sp, cp) = pitch.sin_cos();
    let (sy, cy) = yaw.sin_cos();
    [
        [
            cy * cp,
            cy * sp * sr - sy * cr,
            cy * sp * cr + sy * sr,
            translation[0],
        ],
        [
            sy * cp,
            sy * sp * sr + cy * cr,
            sy * sp * cr - cy * sr,
            translation[1],
        ],
        [-sp, cp * sr, cp * cr, translation[2]],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

pub fn multiply(a: &Mat4, b: &Mat4) -> Mat4 {
    let mut out = [[0.0f64; 4]; 4];
    for i in 0..4 {
        for j in 0..4 {
            out[i][j] =
                a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j] + a[i][3] * b[3][j];
        }
    }
    out
}

pub fn translation_of(m: &Mat4) -> [f64; 3] {
    [m[0][3], m[1][3], m[2][3]]
}

/// Rodrigues rotation about a unit axis by `angle` radians, embedded in a Mat4.
/// Used to bake home-pose joint angles into link bind transforms.
pub fn rotation_about_axis(axis: [f64; 3], angle: f64) -> Mat4 {
    let len_sq = axis[0] * axis[0] + axis[1] * axis[1] + axis[2] * axis[2];
    if len_sq < 1e-12 {
        return IDENTITY;
    }
    let inv_len = 1.0 / len_sq.sqrt();
    let (x, y, z) = (axis[0] * inv_len, axis[1] * inv_len, axis[2] * inv_len);
    let (s, c) = angle.sin_cos();
    let one_c = 1.0 - c;
    [
        [
            c + x * x * one_c,
            x * y * one_c - z * s,
            x * z * one_c + y * s,
            0.0,
        ],
        [
            y * x * one_c + z * s,
            c + y * y * one_c,
            y * z * one_c - x * s,
            0.0,
        ],
        [
            z * x * one_c - y * s,
            z * y * one_c + x * s,
            c + z * z * one_c,
            0.0,
        ],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

/// Convert the 3x3 rotation sub-matrix of `m` to a unit quaternion `[x, y, z, w]`,
/// canonicalised so `w >= 0`.
pub fn quat_of(m: &Mat4) -> [f64; 4] {
    let trace = m[0][0] + m[1][1] + m[2][2];
    let (qx, qy, qz, qw) = if trace > 0.0 {
        let s = (trace + 1.0).sqrt() * 2.0;
        (
            (m[2][1] - m[1][2]) / s,
            (m[0][2] - m[2][0]) / s,
            (m[1][0] - m[0][1]) / s,
            0.25 * s,
        )
    } else if m[0][0] > m[1][1] && m[0][0] > m[2][2] {
        let s = (1.0 + m[0][0] - m[1][1] - m[2][2]).sqrt() * 2.0;
        (
            0.25 * s,
            (m[0][1] + m[1][0]) / s,
            (m[0][2] + m[2][0]) / s,
            (m[2][1] - m[1][2]) / s,
        )
    } else if m[1][1] > m[2][2] {
        let s = (1.0 + m[1][1] - m[0][0] - m[2][2]).sqrt() * 2.0;
        (
            (m[0][1] + m[1][0]) / s,
            0.25 * s,
            (m[1][2] + m[2][1]) / s,
            (m[0][2] - m[2][0]) / s,
        )
    } else {
        let s = (1.0 + m[2][2] - m[0][0] - m[1][1]).sqrt() * 2.0;
        (
            (m[0][2] + m[2][0]) / s,
            (m[1][2] + m[2][1]) / s,
            0.25 * s,
            (m[1][0] - m[0][1]) / s,
        )
    };
    let mag = (qx * qx + qy * qy + qz * qz + qw * qw).sqrt();
    let (qx, qy, qz, qw) = if mag < 1e-12 {
        (0.0, 0.0, 0.0, 1.0)
    } else {
        (qx / mag, qy / mag, qz / mag, qw / mag)
    };
    // Canonicalise: PhysX prefers w >= 0
    if qw < 0.0 {
        [-qx, -qy, -qz, -qw]
    } else {
        [qx, qy, qz, qw]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_round_trip() {
        let q = quat_of(&IDENTITY);
        assert!((q[0]).abs() < 1e-9);
        assert!((q[1]).abs() < 1e-9);
        assert!((q[2]).abs() < 1e-9);
        assert!((q[3] - 1.0).abs() < 1e-9);
    }

    #[test]
    fn translation_only() {
        let m = from_translation_rpy([1.0, 2.0, 3.0], [0.0, 0.0, 0.0]);
        assert_eq!(translation_of(&m), [1.0, 2.0, 3.0]);
        let q = quat_of(&m);
        assert!((q[3] - 1.0).abs() < 1e-9);
    }

    #[test]
    fn yaw_90_deg() {
        let m = from_translation_rpy([0.0; 3], [0.0, 0.0, std::f64::consts::FRAC_PI_2]);
        let q = quat_of(&m);
        // 90° around Z: w = cos(45°) = sqrt(2)/2, z = sin(45°) = sqrt(2)/2
        assert!((q[2] - 0.7071067811865475).abs() < 1e-9);
        assert!((q[3] - 0.7071067811865475).abs() < 1e-9);
    }

    #[test]
    fn quat_w_negative_is_canonicalised() {
        // Build a rotation that yields w<0 then verify quat_of flips it.
        // 270° around Z: cos(135°) ≈ -0.707, sin(135°) ≈ 0.707
        let angle = 3.0 * std::f64::consts::FRAC_PI_2;
        let m = from_translation_rpy([0.0; 3], [0.0, 0.0, angle]);
        let q = quat_of(&m);
        assert!(q[3] >= 0.0, "quat_of must canonicalise w >= 0; got w = {}", q[3]);
    }
}
