//! robo-zenoh-bridge: subscribe R3's joint commands over Zenoh and republish
//! them onto Isaac's local ROS 2 / DDS. Runs ON the Isaac host.
//!
//! R3's `ZenohSink` publishes over Zenoh (TCP unicast) to this bridge; the
//! bridge republishes onto DDS over loopback multicast (works because Isaac and
//! the bridge share one host). This is the cross-host / cross-VPC path DDS
//! multicast discovery can't take by itself.
//!
//! Both sides are synchronous: `zenoh::open(..).wait()`, `subscriber.recv()`
//! (blocking flume channel). No async runtime.

use std::net::UdpSocket;

use clap::Parser;
use zenoh::{Config, Wait};

use robo_ros_runner::transport::dds::Ros2ClientSink;
use robo_ros_runner::transport::zenoh::{key_expr_to_topic, ZenohJointCommand, DEFAULT_KEY_PREFIX};
use robo_ros_runner::transport::{CommandSink, JointCommand};
use robo_ros_runner::Stamp;

/// Bridge R3's Zenoh joint commands onto Isaac's local ROS 2 / DDS.
#[derive(Parser, Debug, Clone)]
#[command(
    name = "robo-zenoh-bridge",
    version,
    about = "Bridge R3's Zenoh joint commands onto Isaac's local ROS 2 / DDS"
)]
pub struct Args {
    /// Zenoh listen endpoint(s), e.g. `tcp/0.0.0.0:7447` (repeatable).
    #[arg(long, default_value = "tcp/0.0.0.0:7447")]
    pub listen: Vec<String>,
    /// ROS_DOMAIN_ID to republish onto (Isaac's domain).
    #[arg(long, default_value_t = 0)]
    pub domain: u32,
    /// Key-expr prefix; MUST match R3's `--zenoh-key-prefix`.
    #[arg(long, default_value = DEFAULT_KEY_PREFIX)]
    pub key_prefix: String,
}

/// Best-effort primary IP for the friendly banner. Connecting a UDP socket
/// only selects the routing interface — no packets are sent.
fn primary_ip() -> Option<std::net::IpAddr> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("8.8.8.8:80").ok()?;
    s.local_addr().ok().map(|a| a.ip())
}

fn print_banner(args: &Args) {
    let ip = primary_ip()
        .map(|i| i.to_string())
        .unwrap_or_else(|| "<this-host-ip>".to_string());
    eprintln!(
        "robo-zenoh-bridge listening on {:?}, republishing to DDS domain {}",
        args.listen, args.domain
    );
    eprintln!("On the R3 host, run:");
    eprintln!("    robo-ros-runner run --cell <cell.yaml> --csv <capture.csv> \\");
    eprintln!(
        "        --sink zenoh --zenoh-connect tcp/{}:7447 --zenoh-key-prefix {}",
        ip, args.key_prefix
    );
}

/// Run the bridge: open a Zenoh peer session, subscribe `<prefix>/**`, and
/// republish every received command onto DDS. Blocks forever (until the Zenoh
/// session closes / all senders drop).
pub fn run(args: Args) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut cfg = Config::default();
    cfg.insert_json5("mode", r#""peer""#)?;
    cfg.insert_json5("scouting/multicast/enabled", "false")?;
    cfg.insert_json5("listen/endpoints", &serde_json::to_string(&args.listen)?)?;

    let session = zenoh::open(cfg)
        .wait()
        .map_err(|e| format!("zenoh open: {}", e))?;
    let sub_key = format!("{}/**", args.key_prefix);
    let subscriber = session
        .declare_subscriber(&sub_key)
        .wait()
        .map_err(|e| format!("zenoh subscribe {}: {}", sub_key, e))?;

    // ONE sink for every topic; Ros2ClientSink caches a publisher per topic.
    let mut dds = Ros2ClientSink::new(args.domain)?;

    print_banner(&args);

    loop {
        let sample = subscriber.recv().map_err(|e| format!("recv: {}", e))?;
        let bytes = sample.payload().to_bytes();
        let wire = match ZenohJointCommand::from_bytes(&bytes) {
            Ok(w) => w,
            Err(e) => {
                log::warn!(
                    "drop malformed sample on {}: {}",
                    sample.key_expr().as_str(),
                    e
                );
                continue;
            }
        };
        // The payload topic is authoritative; the key expr must reconstruct to
        // the same string (guards against a key-prefix mismatch between ends).
        debug_assert_eq!(
            wire.topic,
            key_expr_to_topic(&args.key_prefix, sample.key_expr().as_str())
        );
        let cmd = JointCommand {
            topic: &wire.topic,
            names: &wire.names,
            positions: &wire.positions,
            stamp: Stamp {
                sec: wire.sec,
                nanosec: wire.nanosec,
            },
        };
        if let Err(e) = dds.publish_joint_command(&cmd) {
            log::error!("republish {}: {}", wire.topic, e);
        }
    }
}
