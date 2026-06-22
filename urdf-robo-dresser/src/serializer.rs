use std::io::Write;

use crate::types::{Collision, Geometry, Inertial, Joint, Link, Origin, UrdfRobot, Visual};

// FR-010: Deterministic XML serialization (NFR-003)
pub fn serialize_urdf(robot: &UrdfRobot) -> Vec<u8> {
    let mut buf = Vec::with_capacity(4096);
    write_urdf(robot, &mut buf).expect("write to Vec<u8> cannot fail");
    buf
}

fn write_urdf(robot: &UrdfRobot, buf: &mut Vec<u8>) -> std::io::Result<()> {
    writeln!(buf, "<?xml version=\"1.0\" encoding=\"UTF-8\"?>")?;
    writeln!(buf, "<robot name=\"{}\">", xml_escape(&robot.name))?;

    for link in &robot.links {
        write_link(link, buf)?;
    }
    for joint in &robot.joints {
        write_joint(joint, buf)?;
    }

    writeln!(buf, "</robot>")?;
    Ok(())
}

fn write_link(link: &Link, buf: &mut Vec<u8>) -> std::io::Result<()> {
    if link.inertial.is_none() && link.visuals.is_empty() && link.collisions.is_empty() {
        writeln!(buf, "  <link name=\"{}\"/>", xml_escape(&link.name))?;
        return Ok(());
    }

    writeln!(buf, "  <link name=\"{}\">", xml_escape(&link.name))?;

    if let Some(ref inertial) = link.inertial {
        write_inertial(inertial, buf)?;
    }
    for visual in &link.visuals {
        write_visual(visual, buf)?;
    }
    for collision in &link.collisions {
        write_collision(collision, buf)?;
    }

    writeln!(buf, "  </link>")?;
    Ok(())
}

fn write_inertial(inertial: &Inertial, buf: &mut Vec<u8>) -> std::io::Result<()> {
    writeln!(buf, "    <inertial>")?;
    write_origin(&inertial.origin, buf, 6)?;
    writeln!(buf, "      <mass value=\"{}\"/>", fmt_f64(inertial.mass))?;
    writeln!(
        buf,
        "      <inertia ixx=\"{}\" ixy=\"{}\" ixz=\"{}\" iyy=\"{}\" iyz=\"{}\" izz=\"{}\"/>",
        fmt_f64(inertial.inertia.ixx),
        fmt_f64(inertial.inertia.ixy),
        fmt_f64(inertial.inertia.ixz),
        fmt_f64(inertial.inertia.iyy),
        fmt_f64(inertial.inertia.iyz),
        fmt_f64(inertial.inertia.izz),
    )?;
    writeln!(buf, "    </inertial>")?;
    Ok(())
}

fn write_visual(visual: &Visual, buf: &mut Vec<u8>) -> std::io::Result<()> {
    writeln!(buf, "    <visual>")?;
    write_origin(&visual.origin, buf, 6)?;
    writeln!(buf, "      <geometry>")?;
    write_geometry(&visual.geometry, buf)?;
    writeln!(buf, "      </geometry>")?;
    writeln!(buf, "    </visual>")?;
    Ok(())
}

fn write_collision(collision: &Collision, buf: &mut Vec<u8>) -> std::io::Result<()> {
    writeln!(buf, "    <collision>")?;
    write_origin(&collision.origin, buf, 6)?;
    writeln!(buf, "      <geometry>")?;
    write_geometry(&collision.geometry, buf)?;
    writeln!(buf, "      </geometry>")?;
    writeln!(buf, "    </collision>")?;
    Ok(())
}

fn write_geometry(geom: &Geometry, buf: &mut Vec<u8>) -> std::io::Result<()> {
    match geom {
        Geometry::Mesh { filename, scale } => {
            writeln!(
                buf,
                "        <mesh filename=\"{}\" scale=\"{} {} {}\"/>",
                xml_escape(filename),
                fmt_f64(scale[0]),
                fmt_f64(scale[1]),
                fmt_f64(scale[2]),
            )?;
        }
        Geometry::Box { size } => {
            writeln!(
                buf,
                "        <box size=\"{} {} {}\"/>",
                fmt_f64(size[0]),
                fmt_f64(size[1]),
                fmt_f64(size[2]),
            )?;
        }
        Geometry::Cylinder { radius, length } => {
            writeln!(
                buf,
                "        <cylinder radius=\"{}\" length=\"{}\"/>",
                fmt_f64(*radius),
                fmt_f64(*length),
            )?;
        }
        Geometry::Sphere { radius } => {
            writeln!(buf, "        <sphere radius=\"{}\"/>", fmt_f64(*radius))?;
        }
    }
    Ok(())
}

fn write_joint(joint: &Joint, buf: &mut Vec<u8>) -> std::io::Result<()> {
    writeln!(
        buf,
        "  <joint name=\"{}\" type=\"{}\">",
        xml_escape(&joint.name),
        joint.joint_type.as_str(),
    )?;
    write_origin(&joint.origin, buf, 4)?;
    writeln!(buf, "    <parent link=\"{}\"/>", xml_escape(&joint.parent))?;
    writeln!(buf, "    <child link=\"{}\"/>", xml_escape(&joint.child))?;
    writeln!(
        buf,
        "    <axis xyz=\"{} {} {}\"/>",
        fmt_f64(joint.axis[0]),
        fmt_f64(joint.axis[1]),
        fmt_f64(joint.axis[2]),
    )?;
    if let Some(ref limit) = joint.limit {
        writeln!(
            buf,
            "    <limit lower=\"{}\" upper=\"{}\" effort=\"{}\" velocity=\"{}\"/>",
            fmt_f64(limit.lower),
            fmt_f64(limit.upper),
            fmt_f64(limit.effort),
            fmt_f64(limit.velocity),
        )?;
    }
    writeln!(buf, "  </joint>")?;
    Ok(())
}

fn write_origin(origin: &Origin, buf: &mut Vec<u8>, indent: usize) -> std::io::Result<()> {
    let pad: String = " ".repeat(indent);
    writeln!(
        buf,
        "{}<origin xyz=\"{} {} {}\" rpy=\"{} {} {}\"/>",
        pad,
        fmt_f64(origin.xyz[0]),
        fmt_f64(origin.xyz[1]),
        fmt_f64(origin.xyz[2]),
        fmt_f64(origin.rpy[0]),
        fmt_f64(origin.rpy[1]),
        fmt_f64(origin.rpy[2]),
    )?;
    Ok(())
}

fn fmt_f64(v: f64) -> String {
    if v == 0.0 {
        return "0".to_string();
    }
    let s = format!("{:.10}", v);
    let s = s.trim_end_matches('0');
    let s = s.trim_end_matches('.');
    s.to_string()
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_urdf;
    use std::path::Path;

    #[test]
    fn test_fmt_f64() {
        assert_eq!(fmt_f64(0.0), "0");
        assert_eq!(fmt_f64(1.0), "1");
        assert_eq!(fmt_f64(3.1415927), "3.1415927");
        assert_eq!(fmt_f64(0.860625), "0.860625");
        assert_eq!(fmt_f64(-0.052275), "-0.052275");
    }

    #[test]
    fn test_round_trip_robot() {
        let robot_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../assets/urdf-robot-collection");
        let urdf_path = match std::fs::read_dir(&robot_dir)
            .ok()
            .and_then(|rd| rd.filter_map(|e| e.ok()).map(|e| e.path())
                .find(|p| p.extension().map(|x| x == "urdf").unwrap_or(false)))
        {
            Some(p) => p,
            None => return,
        };

        let robot1 = parse_urdf(&urdf_path).expect("parse failed");
        let xml1 = serialize_urdf(&robot1);
        let robot2 =
            crate::parser::parse_urdf_bytes(&xml1, "round-trip").expect("re-parse failed");

        assert_eq!(robot1.name, robot2.name);
        assert_eq!(robot1.links.len(), robot2.links.len());
        assert_eq!(robot1.joints.len(), robot2.joints.len());

        for (l1, l2) in robot1.links.iter().zip(robot2.links.iter()) {
            assert_eq!(l1.name, l2.name);
            assert_eq!(l1.visuals.len(), l2.visuals.len());
            assert_eq!(l1.collisions.len(), l2.collisions.len());
        }

        for (j1, j2) in robot1.joints.iter().zip(robot2.joints.iter()) {
            assert_eq!(j1.name, j2.name);
            assert_eq!(j1.joint_type, j2.joint_type);
            assert_eq!(j1.parent, j2.parent);
            assert_eq!(j1.child, j2.child);
        }
    }
}
