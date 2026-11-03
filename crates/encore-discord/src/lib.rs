//! Discord Rich Presence through the desktop client's local IPC (D31).
//!
//! Discord listens on `discord-ipc-0` to `-9`: a Unix socket in the
//! runtime or temporary folder (also inside the Flatpak and Snap sandboxes'
//! folders) on macOS and Linux, a named pipe on Windows. Each message is a
//! frame: an opcode and a length (little-endian `u32`s), then JSON. Encore
//! says hello with its application id (public, not a secret), waits for
//! `READY`, and then only ever sends `SET_ACTIVITY`. Nothing goes over the
//! network from here; Discord shows the activity to the user's friends.

use std::io::{self, Read, Write};

use serde_json::{Value, json};

const HANDSHAKE: u32 = 0;
const FRAME: u32 = 1;
const CLOSE: u32 = 2;
const PING: u32 = 3;
const PONG: u32 = 4;

/// What Encore shows: "Listening to Encore", the song, the artist, and a
/// bar from start to end while it plays.
#[derive(Clone, Debug, PartialEq)]
pub struct Activity {
    /// The song.
    pub details: String,
    /// The artist.
    pub state: String,
    /// When the song (as if played without pauses) started, unix ms.
    pub start_ms: i64,
    /// When it ends, if its length is known.
    pub end_ms: Option<i64>,
    /// An https cover URL (allow-listed hosts only, D14).
    pub image: Option<String>,
    /// Shown over the cover: the album.
    pub image_text: Option<String>,
}

/// Discord wants 2 to 128 characters in its text fields.
fn field(s: &str) -> String {
    let mut s: String = s.trim().chars().take(128).collect();
    while s.len() > 128 {
        s.pop();
    }
    if s.chars().count() < 2 {
        s.push_str(if s.is_empty() { "  " } else { " " });
    }
    s
}

impl Activity {
    pub fn to_json(&self) -> Value {
        let mut timestamps = json!({ "start": self.start_ms });
        if let Some(e) = self.end_ms {
            timestamps["end"] = json!(e);
        }
        let mut a = json!({
            // 2: "Listening to".
            "type": 2,
            "details": field(&self.details),
            "state": field(&self.state),
            "timestamps": timestamps,
            "instance": false,
        });
        if let Some(img) = &self.image {
            let mut assets = json!({ "large_image": img });
            if let Some(t) = &self.image_text {
                assets["large_text"] = json!(field(t));
            }
            a["assets"] = assets;
        }
        a
    }
}

pub fn encode(op: u32, payload: &Value) -> Vec<u8> {
    let body = payload.to_string().into_bytes();
    let mut out = Vec::with_capacity(8 + body.len());
    out.extend_from_slice(&op.to_le_bytes());
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    out
}

pub fn read_frame(r: &mut impl Read) -> io::Result<(u32, Value)> {
    let mut head = [0u8; 8];
    r.read_exact(&mut head)?;
    let op = u32::from_le_bytes(head[..4].try_into().unwrap());
    let len = u32::from_le_bytes(head[4..].try_into().unwrap()) as usize;
    if len > 64 * 1024 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "frame too large"));
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body)?;
    let v = serde_json::from_slice(&body).map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "not JSON"))?;
    Ok((op, v))
}

pub trait Pipe: Read + Write + Send {}
impl<T: Read + Write + Send> Pipe for T {}

/// A connection to the Discord client.
pub struct Ipc {
    pipe: Box<dyn Pipe>,
    nonce: u64,
}

impl Ipc {
    /// Find a running Discord and say hello.
    pub fn connect(client_id: &str) -> io::Result<Ipc> {
        let mut last = io::Error::new(io::ErrorKind::NotFound, "Discord isn't running");
        for p in candidates() {
            match open(&p) {
                Ok(pipe) => match Ipc::handshake(pipe, client_id) {
                    Ok(ipc) => return Ok(ipc),
                    Err(e) => last = e,
                },
                Err(e) if e.kind() == io::ErrorKind::NotFound || e.kind() == io::ErrorKind::ConnectionRefused => {}
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    /// Hello on an open pipe, then wait for `READY`.
    pub fn handshake(mut pipe: Box<dyn Pipe>, client_id: &str) -> io::Result<Ipc> {
        pipe.write_all(&encode(HANDSHAKE, &json!({ "v": 1, "client_id": client_id })))?;
        loop {
            let (op, v) = read_frame(&mut pipe)?;
            match op {
                FRAME if v.get("evt").and_then(Value::as_str) == Some("READY") => return Ok(Ipc { pipe, nonce: 0 }),
                PING => pipe.write_all(&encode(PONG, &v))?,
                CLOSE => return Err(io::Error::new(io::ErrorKind::ConnectionRefused, close_reason(&v))),
                _ => {}
            }
        }
    }

    /// Show `a`, or clear the activity with `None`.
    pub fn set_activity(&mut self, a: Option<&Activity>) -> io::Result<()> {
        self.nonce += 1;
        let nonce = self.nonce.to_string();
        let msg = json!({
            "cmd": "SET_ACTIVITY",
            "args": { "pid": std::process::id(), "activity": a.map(Activity::to_json) },
            "nonce": nonce,
        });
        self.pipe.write_all(&encode(FRAME, &msg))?;
        loop {
            let (op, v) = read_frame(&mut self.pipe)?;
            match op {
                PING => self.pipe.write_all(&encode(PONG, &v))?,
                CLOSE => return Err(io::Error::new(io::ErrorKind::ConnectionAborted, close_reason(&v))),
                FRAME if v.get("nonce").and_then(Value::as_str) == Some(nonce.as_str()) => {
                    if v.get("evt").and_then(Value::as_str) == Some("ERROR") {
                        let m = v.pointer("/data/message").and_then(Value::as_str).unwrap_or("Discord refused the activity");
                        return Err(io::Error::other(m.to_string()));
                    }
                    return Ok(());
                }
                _ => {}
            }
        }
    }

    /// Say goodbye; Discord clears the activity.
    pub fn close(mut self) {
        let _ = self.pipe.write_all(&encode(CLOSE, &json!({})));
    }
}

fn close_reason(v: &Value) -> String {
    v.get("message").and_then(Value::as_str).unwrap_or("Discord closed the connection").to_string()
}

#[cfg(unix)]
fn candidates() -> Vec<std::path::PathBuf> {
    let mut dirs: Vec<std::path::PathBuf> = ["XDG_RUNTIME_DIR", "TMPDIR", "TMP", "TEMP"].iter().filter_map(|k| std::env::var_os(k)).map(Into::into).collect();
    dirs.push("/tmp".into());
    let mut out = vec![];
    for d in dirs {
        // The plain client, then the Flatpak and Snap ones.
        for sub in ["", "app/com.discordapp.Discord", "snap.discord", ".flatpak/dev.vencord.Vesktop/xdg-run"] {
            for i in 0..10 {
                let p = d.join(sub).join(format!("discord-ipc-{i}"));
                if !out.contains(&p) {
                    out.push(p);
                }
            }
        }
    }
    out
}

#[cfg(windows)]
fn candidates() -> Vec<std::path::PathBuf> {
    (0..10).map(|i| format!(r"\\.\pipe\discord-ipc-{i}").into()).collect()
}

#[cfg(unix)]
fn open(p: &std::path::Path) -> io::Result<Box<dyn Pipe>> {
    let s = std::os::unix::net::UnixStream::connect(p)?;
    s.set_read_timeout(Some(std::time::Duration::from_secs(5)))?;
    s.set_write_timeout(Some(std::time::Duration::from_secs(5)))?;
    Ok(Box::new(s))
}

#[cfg(windows)]
fn open(p: &std::path::Path) -> io::Result<Box<dyn Pipe>> {
    Ok(Box::new(std::fs::OpenOptions::new().read(true).write(true).open(p)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song() -> Activity {
        Activity { details: "Daybreak Signal".into(), state: "Lumen Drive".into(), start_ms: 1_800_000_000_000, end_ms: Some(1_800_000_201_000), image: Some("https://lh3.googleusercontent.com/abc=w544-h544".into()), image_text: Some("Night Roads".into()) }
    }

    #[test]
    fn frames_round_trip() {
        let v = json!({ "a": 1 });
        let bytes = encode(FRAME, &v);
        assert_eq!(&bytes[..4], &1u32.to_le_bytes());
        assert_eq!(read_frame(&mut &bytes[..]).unwrap(), (FRAME, v));
    }

    #[test]
    fn the_activity_says_listening() {
        let j = song().to_json();
        assert_eq!(j["type"], 2);
        assert_eq!(j["details"], "Daybreak Signal");
        assert_eq!(j["timestamps"]["end"], 1_800_000_201_000i64);
        assert_eq!(j["assets"]["large_text"], "Night Roads");
        let short = Activity { details: "X".into(), state: "".into(), end_ms: None, image: None, ..song() }.to_json();
        assert_eq!(short["details"], "X ", "Discord wants two characters at least");
        assert!(short.get("assets").is_none() && short["timestamps"].get("end").is_none());
        assert_eq!(field(&"é".repeat(200)).chars().count(), 64, "128 bytes at most");
    }

    #[cfg(unix)]
    #[test]
    fn talks_to_a_discord() {
        use std::os::unix::net::UnixListener;
        let dir = std::env::temp_dir().join(format!("encore-discord-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("discord-ipc-0");
        let _ = std::fs::remove_file(&path);
        let l = UnixListener::bind(&path).unwrap();
        let server = std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let (op, hello) = read_frame(&mut s).unwrap();
            assert_eq!((op, hello["client_id"].as_str()), (HANDSHAKE, Some("123")));
            s.write_all(&encode(PING, &json!({}))).unwrap();
            assert_eq!(read_frame(&mut s).unwrap().0, PONG);
            s.write_all(&encode(FRAME, &json!({ "cmd": "DISPATCH", "evt": "READY", "data": {} }))).unwrap();
            let mut seen = vec![];
            for _ in 0..2 {
                let (_, m) = read_frame(&mut s).unwrap();
                seen.push(m["args"]["activity"].clone());
                let evt = if seen.len() == 2 { json!("ERROR") } else { Value::Null };
                s.write_all(&encode(FRAME, &json!({ "cmd": "SET_ACTIVITY", "evt": evt, "nonce": m["nonce"], "data": { "message": "nope" } }))).unwrap();
            }
            seen
        });
        let mut ipc = Ipc::handshake(open(&path).unwrap(), "123").unwrap();
        ipc.set_activity(Some(&song())).unwrap();
        let err = ipc.set_activity(None).unwrap_err();
        assert_eq!(err.to_string(), "nope");
        let seen = server.join().unwrap();
        assert_eq!(seen[0]["details"], "Daybreak Signal");
        assert!(seen[1].is_null(), "None clears");
        let _ = std::fs::remove_dir_all(dir);
    }
}
