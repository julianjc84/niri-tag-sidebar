use serde::Deserialize;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::sync::mpsc;

/// Minimal subset of niri's Event enum — only the gesture variants we care about.
/// Niri uses serde's default externally-tagged enum encoding, so JSON looks like:
/// `{"GestureBegin": {"tag": "sidebar", "trigger": "TouchEdgeLeft", ...}}`
#[derive(Debug, Deserialize, Clone)]
#[allow(dead_code)]
pub enum Event {
    GestureBegin {
        tag: String,
        trigger: String,
        finger_count: u8,
        is_continuous: bool,
    },
    GestureProgress {
        tag: String,
        progress: f64,
        /// Typed physical delta (Swipe { dx, dy } / Pinch { d_spread } /
        /// Rotate { d_radians }). niri-tag-sidebar only reads `progress`,
        /// so we parse it as a raw value and never inspect it.
        delta: serde_json::Value,
        timestamp_ms: u32,
    },
    GestureEnd {
        tag: String,
        completed: bool,
    },
}

/// Messages sent from the IPC thread to the GTK main loop.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum GestureMsg {
    Begin {
        tag: String,
        trigger: String,
        finger_count: u8,
        is_continuous: bool,
    },
    Progress {
        tag: String,
        progress: f64,
    },
    End {
        tag: String,
        completed: bool,
    },
    /// IPC connection lost or error.
    Disconnected(String),
}

/// Niri IPC request — we only need EventStream.
#[derive(serde::Serialize)]
enum Request {
    EventStream,
}

/// Niri IPC reply wrapper.
#[derive(Deserialize)]
#[serde(untagged)]
enum Reply {
    Ok(()),
    Err(ReplyErr),
}


#[derive(Deserialize)]
struct ReplyErr {
    #[serde(rename = "Err")]
    err: String,
}

/// Spawn a background thread that connects to niri IPC, requests the event stream,
/// and sends gesture events over the returned channel.
///
/// `tags` is the set of tags we care about — events for other tags are dropped.
pub fn spawn_ipc_listener(tags: Vec<String>) -> mpsc::Receiver<GestureMsg> {
    let (tx, rx) = mpsc::channel();

    std::thread::Builder::new()
        .name("niri-ipc".to_string())
        .spawn(move || {
            if let Err(e) = ipc_thread(&tx, &tags) {
                let _ = tx.send(GestureMsg::Disconnected(e));
            }
        })
        .expect("Failed to spawn IPC thread");

    rx
}

fn ipc_thread(tx: &mpsc::Sender<GestureMsg>, tags: &[String]) -> Result<(), String> {
    let socket_path = std::env::var("NIRI_SOCKET")
        .map_err(|_| "NIRI_SOCKET not set — are you running inside niri?".to_string())?;

    let mut stream = UnixStream::connect(&socket_path)
        .map_err(|e| format!("Failed to connect to {}: {}", socket_path, e))?;

    // Send EventStream request
    let request = serde_json::to_string(&Request::EventStream)
        .map_err(|e| format!("Failed to serialize request: {}", e))?;
    stream
        .write_all(format!("{}\n", request).as_bytes())
        .map_err(|e| format!("Failed to write to socket: {}", e))?;

    let reader = BufReader::new(stream);

    for line in reader.lines() {
        let line = line.map_err(|e| format!("Socket read error: {}", e))?;

        if line.trim().is_empty() {
            continue;
        }

        // First line is the Reply to our EventStream request
        if let Ok(reply) = serde_json::from_str::<Reply>(&line) {
            match reply {
                Reply::Err(e) => return Err(format!("Niri rejected EventStream: {}", e.err)),
                Reply::Ok(_) => continue,
            }
        }

        // Subsequent lines are Events — try to parse as gesture events.
        // Non-gesture events will fail to match any variant and that's fine.
        let parsed: Result<HashMap<String, serde_json::Value>, _> =
            serde_json::from_str(&line);

        let Ok(map) = parsed else { continue };

        let msg = if let Some(data) = map.get("GestureBegin") {
            let tag = data["tag"].as_str().unwrap_or("").to_string();
            if !tags.contains(&tag) {
                continue;
            }
            Some(GestureMsg::Begin {
                tag,
                trigger: data["trigger"].as_str().unwrap_or("").to_string(),
                finger_count: data["finger_count"].as_u64().unwrap_or(0) as u8,
                is_continuous: data["is_continuous"].as_bool().unwrap_or(false),
            })
        } else if let Some(data) = map.get("GestureProgress") {
            let tag = data["tag"].as_str().unwrap_or("").to_string();
            if !tags.contains(&tag) {
                continue;
            }
            Some(GestureMsg::Progress {
                tag,
                progress: data["progress"].as_f64().unwrap_or(0.0),
            })
        } else if let Some(data) = map.get("GestureEnd") {
            let tag = data["tag"].as_str().unwrap_or("").to_string();
            if !tags.contains(&tag) {
                continue;
            }
            Some(GestureMsg::End {
                tag,
                completed: data["completed"].as_bool().unwrap_or(false),
            })
        } else {
            None
        };

        if let Some(msg) = msg {
            if tx.send(msg).is_err() {
                return Ok(()); // Receiver dropped, app shutting down
            }
        }
    }

    Err("IPC stream ended unexpectedly".to_string())
}
