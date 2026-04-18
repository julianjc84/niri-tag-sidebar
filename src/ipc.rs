use serde::Deserialize;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::sync::mpsc;

/// Minimal subset of niri's Event enum — only the gesture variants we care about.
/// Niri uses serde's default externally-tagged enum encoding, so JSON looks like:
/// `{"GestureBegin": {"tag": "sidebar", "trigger": "TouchEdgeLeft", ...}}`
#[derive(Debug, Deserialize, Clone)]
pub enum Event {
    #[serde(rename = "GestureBegin")]
    Begin {
        tag: String,
        trigger: String,
        finger_count: u8,
        is_continuous: bool,
    },
    #[serde(rename = "GestureProgress")]
    Progress { tag: String, progress: f64 },
    #[serde(rename = "GestureEnd")]
    End { tag: String, completed: bool },
}

/// Messages sent from the IPC thread to the GTK main loop.
#[derive(Debug, Clone)]
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
        // Non-gesture events won't match any variant and that's fine.
        let Ok(event) = serde_json::from_str::<Event>(&line) else {
            continue;
        };

        let tag = match &event {
            Event::Begin { tag, .. } | Event::Progress { tag, .. } | Event::End { tag, .. } => tag,
        };
        if !tags.contains(tag) {
            continue;
        }

        let msg = match event {
            Event::Begin {
                tag,
                trigger,
                finger_count,
                is_continuous,
            } => GestureMsg::Begin {
                tag,
                trigger,
                finger_count,
                is_continuous,
            },
            Event::Progress { tag, progress } => GestureMsg::Progress { tag, progress },
            Event::End { tag, completed } => GestureMsg::End { tag, completed },
        };

        if tx.send(msg).is_err() {
            return Ok(()); // Receiver dropped, app shutting down
        }
    }

    Err("IPC stream ended unexpectedly".to_string())
}
