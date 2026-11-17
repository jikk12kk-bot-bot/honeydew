//! Opt-in control for music assistants (MCP). Socket work never touches the song: commands are validated and applied
//! on the egui thread as one ordinary undoable edit.
//!
//! One listener speaks two protocols, told apart by the first byte:
//! - **MCP over Streamable HTTP** (our own addition: the app is the MCP server itself, on the phones too): `POST /mcp`
//!   with a JSON-RPC 2.0 message (or a batch), authenticated with `Authorization: Bearer <token>` (or `?token=`). The
//!   answers are plain JSON (no event stream); `GET` is 405, as the spec allows.
//! - **The raw JSON requests** of the desktop's Python MCP adapter (`tools/honeydew-mcp`): `{"token": …, "action": …}`,
//!   the connection's write side closed, one JSON answer.
//!
//! It listens on 127.0.0.1 (this device), or with `Allow over Wi-Fi` on every network.
//!
//! A request waits for the next frame. No frame runs while the app is in the background (Android stops drawing it, an
//! assistant in another app on the phone being the usual case), so a request still waiting after a moment is answered
//! on the control thread itself ([`background`]), under the lock every frame holds ([`frame_lock`]): the app is used
//! by one thread at a time.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError, TryLockError};
use std::time::{Duration, Instant};

use eframe::egui;
use honeydew_audio::device::AudioCommand;
use serde::Deserialize;
use serde_json::{Value, json};

use super::HoneydewApp;
use crate::params::ParamKind;
use crate::song::{ChannelKind, Event, LaneKind, PPQ, Song, SynthKind};
use crate::ui::views::SongEdit;

const MAX_REQUEST: u64 = 256 * 1024;
/// An HTTP request's head, at most.
const MAX_HEAD: usize = 16 * 1024;
/// The MCP protocol versions answered as asked; any other gets the first.
const PROTOCOLS: [&str; 3] = ["2025-03-26", "2025-06-18", "2024-11-05"];
const INSTRUCTIONS: &str = "Use get_song before edits. Reuse existing configured VST channels by ID. apply_arrangement changes the open song and is one undo step. For built-in synth sound design, use get_instrument_params before set_instrument_params; these knob changes are not individually undoable. play, stop and seek drive the transport; get_mixer and set_mixer read and set the channels' level, pan, mute and solo. Do not claim to have auditioned the result; ask the user to listen and give feedback.";

struct Request {
    value: Value,
    reply: Sender<Value>,
}

/// The app, for [`background`]: set by every frame while the bridge runs, cleared on exit. Every frame holds the lock
/// for its whole length.
static APP: Mutex<Option<AppPtr>> = Mutex::new(None);
/// When the last frame ended, ms after [`START`].
static LAST_FRAME: AtomicU64 = AtomicU64::new(0);
static START: OnceLock<Instant> = OnceLock::new();
/// A request unanswered this long with no frame since is answered in the background.
const NO_FRAMES: Duration = Duration::from_millis(400);

struct AppPtr(*mut HoneydewApp);

// SAFETY: dereferenced only under `APP`'s lock, which a frame holds while it runs, and never after `forget_app` (on
// exit): one thread at a time uses the app, as if it moved to it and back.
unsafe impl Send for AppPtr {}

fn now_ms() -> u64 {
    START.get_or_init(Instant::now).elapsed().as_millis() as u64
}

/// Held for a frame's length ([`frame_lock`]); notes its end.
pub(super) struct FrameGuard(MutexGuard<'static, Option<AppPtr>>);

impl Drop for FrameGuard {
    fn drop(&mut self) {
        LAST_FRAME.store(now_ms(), Ordering::Release);
    }
}

/// At a frame's start: holds the app away from [`background`] until the frame ends, and keeps where it is for it.
pub(super) fn frame_lock(app: &mut HoneydewApp) -> FrameGuard {
    let mut held = APP.lock().unwrap_or_else(PoisonError::into_inner);
    *held = app.control.is_some().then(|| AppPtr(app));
    FrameGuard(held)
}

/// On exit, before the app goes: [`background`] never reaches it again.
pub(super) fn forget_app() {
    *APP.lock().unwrap_or_else(PoisonError::into_inner) = None;
}

/// No frame runs: the requests waiting are answered here, on the control thread, unless a frame has started.
fn background() {
    let held = match APP.try_lock() {
        Ok(held) => held,
        Err(TryLockError::Poisoned(p)) => p.into_inner(),
        Err(TryLockError::WouldBlock) => return,
    };
    if let Some(app) = held.as_ref() {
        // SAFETY: under the lock no frame runs (each holds it), and the app is alive (`forget_app` clears it on exit,
        // under the lock).
        unsafe { (*app.0).control_background() };
    }
}

pub(super) struct Bridge {
    requests: Receiver<Request>,
    shutdown: Sender<()>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Bridge {
    /// Listens on `port` (every network with `wifi`, else this device only) while `enabled`, with the token saved in
    /// Settings (at least 24 characters). `None`: off, or it could not start.
    pub(super) fn start(ctx: egui::Context, enabled: bool, port: u16, wifi: bool) -> Option<Self> {
        if !enabled {
            return None;
        }
        // The Settings token file is shared with the Python MCP adapter on the desktop.
        // Keep the environment fallback during the migration from the old startup flow.
        let token = crate::settings::load_mcp_token()
            .or_else(|| std::env::var("HONEYDEW_CONTROL_TOKEN").ok())?;
        if token.len() < 24 {
            eprintln!("Honeydew control disabled: the MCP token needs at least 24 characters");
            return None;
        }
        let host = if wifi { "0.0.0.0" } else { "127.0.0.1" };
        let listener = match TcpListener::bind((host, port)) {
            Ok(listener) => listener,
            Err(err) => {
                eprintln!("Honeydew control disabled: cannot bind {host}:{port}: {err}");
                return None;
            }
        };
        let (tx, requests) = mpsc::channel();
        let (shutdown, stop) = mpsc::channel();
        listener.set_nonblocking(true).ok()?;
        let thread = std::thread::Builder::new()
            .name("honeydew-control".into())
            .spawn(move || {
                loop {
                    if stop.try_recv().is_ok() {
                        break;
                    }
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            // macOS and Windows hand the listener's non-blocking mode to the stream.
                            let _ = stream.set_nonblocking(false);
                            let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
                            let _ = stream.set_write_timeout(Some(Duration::from_secs(3)));
                            connection(&mut stream, &token, &tx, &ctx);
                            let _ = stream.flush();
                        }
                        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(25));
                        }
                        Err(_) => break,
                    }
                }
            })
            .ok()?;
        eprintln!("Honeydew control enabled on {host}:{port}");
        Some(Self {
            requests,
            shutdown,
            thread: Some(thread),
        })
    }

    fn drain(&self) -> Vec<Request> {
        self.requests.try_iter().take(32).collect()
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        let _ = self.shutdown.send(());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The address an MCP client connects to: `http://<this device's address on the network, or 127.0.0.1>:<port>/mcp`.
pub fn mcp_url(port: u16, wifi: bool) -> String {
    let host = wifi
        .then(lan_address)
        .flatten()
        .unwrap_or_else(|| "127.0.0.1".to_owned());
    format!("http://{host}:{port}/mcp")
}

/// This device's address on its network: the one a route out would leave from (a UDP "connect" sends nothing).
pub fn lan_address() -> Option<String> {
    let socket = std::net::UdpSocket::bind(("0.0.0.0", 0)).ok()?;
    socket.connect(("8.8.8.8", 80)).ok()?;
    let ip = socket.local_addr().ok()?.ip();
    (!ip.is_unspecified() && !ip.is_loopback()).then(|| ip.to_string())
}

/// One connection: an HTTP request (MCP), or a raw JSON one (the Python adapter).
fn connection(stream: &mut TcpStream, token: &str, tx: &Sender<Request>, ctx: &egui::Context) {
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 4096];
    // The first bytes say which.
    match stream.read(&mut chunk) {
        Ok(n) if n > 0 => bytes.extend_from_slice(&chunk[..n]),
        _ => return,
    }
    let first = bytes.iter().find(|b| !b.is_ascii_whitespace()).copied();
    if first == Some(b'{') {
        let answer = serve(stream, bytes, token, tx, ctx);
        let _ = serde_json::to_writer(&mut *stream, &answer);
        return;
    }
    http(stream, bytes, token, tx, ctx);
}

/// The raw JSON request whose first bytes are `bytes`: read to its end, authenticated, and answered.
fn serve(
    stream: &mut TcpStream,
    mut bytes: Vec<u8>,
    token: &str,
    tx: &Sender<Request>,
    ctx: &egui::Context,
) -> Value {
    let left = (MAX_REQUEST + 1).saturating_sub(bytes.len() as u64);
    if stream.take(left).read_to_end(&mut bytes).is_err() || bytes.len() as u64 > MAX_REQUEST {
        return json!({"error": "request too large or incomplete"});
    }
    let Ok(mut value) = serde_json::from_slice::<Value>(&bytes) else {
        return json!({"error": "invalid JSON"});
    };
    let Some(provided) = value.get("token").and_then(Value::as_str) else {
        return json!({"error": "authentication required"});
    };
    if provided != token {
        return json!({"error": "authentication failed"});
    }
    value
        .as_object_mut()
        .expect("token was present")
        .remove("token");
    ask(value, tx, ctx)
}

/// `value` (an action) handed to the app, and its answer.
fn ask(value: Value, tx: &Sender<Request>, ctx: &egui::Context) -> Value {
    let (reply, receive) = mpsc::channel();
    if tx.send(Request { value, reply }).is_err() {
        return json!({"error": "Honeydew closed"});
    }
    ctx.request_repaint();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match receive.recv_timeout(Duration::from_millis(100)) {
            Ok(answer) => return answer,
            Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {}
        }
        if Instant::now() >= deadline {
            break;
        }
        let idle = now_ms().saturating_sub(LAST_FRAME.load(Ordering::Acquire));
        if idle >= NO_FRAMES.as_millis() as u64 {
            background();
        }
    }
    json!({"error": "Honeydew did not respond"})
}

/// An HTTP request whose first bytes are `bytes`: MCP's Streamable HTTP, JSON answers only.
fn http(
    stream: &mut TcpStream,
    mut bytes: Vec<u8>,
    token: &str,
    tx: &Sender<Request>,
    ctx: &egui::Context,
) {
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        if let Some(at) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            break at + 4;
        }
        if bytes.len() > MAX_HEAD {
            return respond(stream, 431, "Request Header Fields Too Large", None);
        }
        match stream.read(&mut chunk) {
            Ok(n) if n > 0 => bytes.extend_from_slice(&chunk[..n]),
            _ => return,
        }
    };
    let head = String::from_utf8_lossy(&bytes[..head_end]).into_owned();
    let mut start = head.split("\r\n").next().unwrap_or_default().split(' ');
    let (method, target) = (
        start.next().unwrap_or_default(),
        start.next().unwrap_or_default(),
    );
    let header = |name: &str| {
        head.split("\r\n").skip(1).find_map(|line| {
            let (k, v) = line.split_once(':')?;
            k.trim()
                .eq_ignore_ascii_case(name)
                .then(|| v.trim().to_owned())
        })
    };
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path != "/mcp" && path != "/" {
        return respond(stream, 404, "Not Found", None);
    }
    match method {
        "OPTIONS" => return respond(stream, 204, "No Content", None),
        "POST" => {}
        _ => return respond(stream, 405, "Method Not Allowed", None),
    }
    let bearer = header("authorization").and_then(|v| {
        v.strip_prefix("Bearer ")
            .or_else(|| v.strip_prefix("bearer "))
            .map(str::to_owned)
    });
    let in_query = query
        .split('&')
        .find_map(|kv| kv.strip_prefix("token=").map(str::to_owned));
    if bearer.or(in_query).as_deref() != Some(token) {
        return respond(
            stream,
            401,
            "Unauthorized",
            Some(&json!({"error": "authentication required"})),
        );
    }
    let length: u64 = header("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    if length > MAX_REQUEST {
        return respond(stream, 413, "Payload Too Large", None);
    }
    let mut body = bytes[head_end..].to_vec();
    while (body.len() as u64) < length {
        match stream.read(&mut chunk) {
            Ok(n) if n > 0 => body.extend_from_slice(&chunk[..n]),
            _ => return,
        }
    }
    body.truncate(length as usize);
    let Ok(message) = serde_json::from_slice::<Value>(&body) else {
        let error = json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": "Parse error"}});
        return respond(stream, 400, "Bad Request", Some(&error));
    };
    let answer = match message {
        Value::Array(batch) => {
            let answers: Vec<Value> = batch.into_iter().filter_map(|m| rpc(m, tx, ctx)).collect();
            (!answers.is_empty()).then_some(Value::Array(answers))
        }
        one => rpc(one, tx, ctx),
    };
    match answer {
        Some(answer) => respond(stream, 200, "OK", Some(&answer)),
        // Notifications and responses only.
        None => respond(stream, 202, "Accepted", None),
    }
}

fn respond(stream: &mut TcpStream, code: u16, reason: &str, body: Option<&Value>) {
    let body = body.map(|b| b.to_string()).unwrap_or_default();
    let kind = if body.is_empty() {
        ""
    } else {
        "Content-Type: application/json\r\n"
    };
    let head = format!(
        "HTTP/1.1 {code} {reason}\r\n{kind}Content-Length: {}\r\nConnection: close\r\n\
         Access-Control-Allow-Origin: *\r\n\
         Access-Control-Allow-Methods: POST, OPTIONS\r\n\
         Access-Control-Allow-Headers: authorization, content-type, accept, mcp-session-id, mcp-protocol-version\r\n\
         {}\r\n",
        body.len(),
        if code == 401 {
            "WWW-Authenticate: Bearer\r\n"
        } else if code == 405 {
            "Allow: POST, OPTIONS\r\n"
        } else {
            ""
        },
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body.as_bytes());
}

/// One JSON-RPC message: its response, or `None` for a notification (or a response the client sent).
fn rpc(message: Value, tx: &Sender<Request>, ctx: &egui::Context) -> Option<Value> {
    let id = message.get("id").cloned()?;
    // A response the client sent: nothing to answer.
    let method = Some(message.get("method")?.as_str()?);
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    let result = match method {
        Some("initialize") => {
            let asked = params.get("protocolVersion").and_then(Value::as_str);
            let version = asked
                .filter(|v| PROTOCOLS.contains(v))
                .unwrap_or(PROTOCOLS[0]);
            json!({
                "protocolVersion": version,
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": "honeydew", "version": env!("CARGO_PKG_VERSION")},
                "instructions": INSTRUCTIONS,
            })
        }
        Some("ping") => json!({}),
        Some("tools/list") => json!({"tools": tools()}),
        Some("tools/call") => {
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            match tool_action(name, &args) {
                None => {
                    json!({"content": [{"type": "text", "text": format!("Unknown tool: {name}")}], "isError": true})
                }
                Some(action) => {
                    let response = ask(action, tx, ctx);
                    let error = response.get("error").cloned();
                    let shown = match &error {
                        Some(e) => json!({"error": e}),
                        None => response.get("result").cloned().unwrap_or(Value::Null),
                    };
                    json!({"content": [{"type": "text", "text": shown.to_string()}], "isError": error.is_some()})
                }
            }
        }
        _ => {
            return Some(json!({"jsonrpc": "2.0", "id": id,
                "error": {"code": -32601, "message": "Method not found"}}));
        }
    };
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

/// The app action an MCP tool call is (the Python adapter sends the same ones).
fn tool_action(name: &str, args: &Value) -> Option<Value> {
    Some(match name {
        "get_song" => json!({"action": "song"}),
        "list_instruments" => json!({"action": "instruments"}),
        "get_instrument_params" => {
            json!({"action": "instrument_params", "channel_id": args.get("channel_id")})
        }
        "set_instrument_params" => json!({"action": "set_instrument_params", "edit": args}),
        "apply_arrangement" => json!({"action": "arrange", "arrangement": args}),
        "play" => json!({"action": "play"}),
        "stop" => json!({"action": "stop"}),
        "seek" => json!({"action": "seek", "bar": args.get("bar")}),
        "get_mixer" => json!({"action": "mixer"}),
        "set_mixer" => json!({"action": "set_mixer", "changes": args.get("changes")}),
        _ => return None,
    })
}

/// The tools, as MCP lists them (tools/honeydew-mcp/server.py lists the same).
fn tools() -> Value {
    serde_json::from_str(TOOLS).expect("the tools are JSON")
}

/// The tools' list ([`tools`]).
const TOOLS: &str = r#"[
 {
  "name": "get_song",
  "description": "Inspect Honeydew's currently open song, channel IDs, configured instruments/VSTs, drum pad indices, and clip IDs. Read this before composing or replacing a clip.",
  "inputSchema": {
   "type": "object",
   "properties": {},
   "additionalProperties": false
  }
 },
 {
  "name": "list_instruments",
  "description": "List Honeydew's built-in synths and installed instrument plugins. Existing configured plugin tracks should be reused by channel ID; creating new external plugin instances is not supported yet.",
  "inputSchema": {
   "type": "object",
   "properties": {},
   "additionalProperties": false
  }
 },
 {
  "name": "get_instrument_params",
  "description": "Read built-in synth modules and their parameter IDs, names, normalized values (0-1), defaults and display text on an existing channel. Call get_song first. External plugin state is not exposed.",
  "inputSchema": {
   "type": "object",
   "properties": {
    "channel_id": {
     "type": "integer"
    }
   },
   "required": [
    "channel_id"
   ],
   "additionalProperties": false
  }
 },
 {
  "name": "set_instrument_params",
  "description": "Adjust 1-32 normalized parameters of one existing built-in synth module. Call get_instrument_params first for its module_id and indexes. Values must be 0-1. All changes validate before applying; they update live audio and song state. Honeydew's existing synth knob edits are not individually undoable. Does not edit external VST/CLAP presets.",
  "inputSchema": {
   "type": "object",
   "properties": {
    "channel_id": {
     "type": "integer"
    },
    "module_id": {
     "type": "integer"
    },
    "changes": {
     "type": "array",
     "minItems": 1,
     "maxItems": 32,
     "items": {
      "type": "object",
      "properties": {
       "index": {
        "type": "integer",
        "minimum": 0
       },
       "value": {
        "type": "number",
        "minimum": 0,
        "maximum": 1
       }
      },
      "required": [
       "index",
       "value"
      ],
      "additionalProperties": false
     }
    }
   },
   "required": [
    "channel_id",
    "module_id",
    "changes"
   ],
   "additionalProperties": false
  }
 },
 {
  "name": "apply_arrangement",
  "description": "Add or replace 1-16 note/drum clips and optionally set tempo in Honeydew's open song as one undoable edit. Use an existing channel ID to preserve its configured VST sound; use a new built-in synth or Drums for a new channel. Call get_song first. Bars are zero-based; note beats are quarter-note positions relative to each clip. For drums, key is the pad index returned by get_song. To revise a clip, pass replace_clip_id and the existing start_bar/bars. Does not change plugin presets or render audio.",
  "inputSchema": {
   "type": "object",
   "properties": {
    "tempo_bpm": {
     "type": "number",
     "minimum": 1,
     "maximum": 999
    },
    "tracks": {
     "type": "array",
     "minItems": 1,
     "maxItems": 16,
     "items": {
      "type": "object",
      "properties": {
       "target": {
        "oneOf": [
         {
          "type": "object",
          "properties": {
           "kind": {
            "const": "existing"
           },
           "channel_id": {
            "type": "integer"
           },
           "replace_clip_id": {
            "type": "integer",
            "minimum": 1
           }
          },
          "required": [
           "kind",
           "channel_id"
          ],
          "additionalProperties": false
         },
         {
          "type": "object",
          "properties": {
           "kind": {
            "const": "new"
           },
           "instrument": {
            "type": "string"
           },
           "name": {
            "type": "string"
           }
          },
          "required": [
           "kind",
           "instrument"
          ],
          "additionalProperties": false
         }
        ]
       },
       "start_bar": {
        "type": "integer",
        "minimum": 0,
        "maximum": 4095
       },
       "bars": {
        "type": "integer",
        "minimum": 1,
        "maximum": 16
       },
       "notes": {
        "type": "array",
        "maxItems": 512,
        "items": {
         "type": "object",
         "properties": {
          "beat": {
           "type": "number",
           "minimum": 0
          },
          "length_beats": {
           "type": "number",
           "exclusiveMinimum": 0
          },
          "key": {
           "type": "integer",
           "minimum": 0,
           "maximum": 127
          },
          "velocity": {
           "type": "integer",
           "minimum": 1,
           "maximum": 255
          }
         },
         "required": [
          "beat",
          "length_beats",
          "key",
          "velocity"
         ],
         "additionalProperties": false
        }
       }
      },
      "required": [
       "target",
       "start_bar",
       "bars",
       "notes"
      ],
      "additionalProperties": false
     }
    }
   },
   "required": [
    "tracks"
   ],
   "additionalProperties": false
  }
 },
 {
  "name": "play",
  "description": "Start Honeydew's playback from the current song position (as the Play button does).",
  "inputSchema": {
   "type": "object",
   "properties": {},
   "additionalProperties": false
  }
 },
 {
  "name": "stop",
  "description": "Stop Honeydew's playback (as the Stop button does); the position stays where it is.",
  "inputSchema": {
   "type": "object",
   "properties": {},
   "additionalProperties": false
  }
 },
 {
  "name": "seek",
  "description": "Move the song position to a zero-based bar (fractions allowed: 1.5 is half way through the second bar). Playback continues from there if it is playing.",
  "inputSchema": {
   "type": "object",
   "properties": {
    "bar": {
     "type": "number",
     "minimum": 0,
     "maximum": 4096
    }
   },
   "required": [
    "bar"
   ],
   "additionalProperties": false
  }
 },
 {
  "name": "get_mixer",
  "description": "Read the mixer: each channel's ID (0 is the master), name, level and pan (normalized 0-1, with their display text), mute and solo.",
  "inputSchema": {
   "type": "object",
   "properties": {},
   "additionalProperties": false
  }
 },
 {
  "name": "set_mixer",
  "description": "Set 1-32 mixer changes: per channel ID (0 is the master), any of level (0-1, the default 0.8531 is 0 dB), pan (0-1, 0.5 centred), mute and solo. All changes validate before applying. Not individually undoable, as mixer moves are not.",
  "inputSchema": {
   "type": "object",
   "properties": {
    "changes": {
     "type": "array",
     "minItems": 1,
     "maxItems": 32,
     "items": {
      "type": "object",
      "properties": {
       "channel_id": {
        "type": "integer"
       },
       "level": {
        "type": "number",
        "minimum": 0,
        "maximum": 1
       },
       "pan": {
        "type": "number",
        "minimum": 0,
        "maximum": 1
       },
       "mute": {
        "type": "boolean"
       },
       "solo": {
        "type": "boolean"
       }
      },
      "required": [
       "channel_id"
      ],
      "additionalProperties": false
     }
    }
   },
   "required": [
    "changes"
   ],
   "additionalProperties": false
  }
 }
]"#;

/// One `set_mixer` change.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MixerChange {
    channel_id: i32,
    level: Option<f64>,
    pan: Option<f64>,
    mute: Option<bool>,
    solo: Option<bool>,
}

/// Validates every change before touching the song; returns each (channel index, param index, value) set.
fn set_mixer(
    song: &mut Song,
    changes: Vec<MixerChange>,
) -> Result<Vec<(usize, usize, f32)>, String> {
    if changes.is_empty() || changes.len() > 32 {
        return Err("provide 1–32 mixer changes".into());
    }
    let mut set = Vec::new();
    for change in &changes {
        let channel = (song.channels.iter())
            .position(|ch| ch.id == change.channel_id)
            .ok_or_else(|| format!("channel {} is not in the open song", change.channel_id))?;
        for (index, value) in [(0, change.level), (1, change.pan)] {
            if let Some(v) = value {
                if !v.is_finite() || !(0.0..=1.0).contains(&v) {
                    return Err("level and pan must be from 0 to 1".into());
                }
                set.push((channel, index, v as f32));
            }
        }
        for (index, on) in [(2, change.mute), (3, change.solo)] {
            if let Some(on) = on {
                set.push((channel, index, if on { 1.0 } else { 0.0 }));
            }
        }
    }
    if set.is_empty() {
        return Err("each change needs level, pan, mute or solo".into());
    }
    for &(channel, index, value) in &set {
        song.channels[channel].params.0[index] = value;
    }
    Ok(set)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Arrangement {
    tempo_bpm: Option<f64>,
    tracks: Vec<TrackPattern>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TrackPattern {
    target: Target,
    /// Zero-based bar in the song.
    start_bar: u32,
    bars: u32,
    notes: Vec<Note>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Target {
    Existing {
        channel_id: i32,
        replace_clip_id: Option<u32>,
    },
    New {
        instrument: String,
        name: Option<String>,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Note {
    /// Position from the start of this clip, measured in quarter-note beats.
    beat: f64,
    length_beats: f64,
    /// MIDI key for a pitched instrument; drum pad index for a drum kit.
    key: u8,
    velocity: u8,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ParameterEdit {
    index: usize,
    value: f64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetInstrumentParams {
    channel_id: i32,
    module_id: i32,
    changes: Vec<ParameterEdit>,
}

fn built_in_instrument(type_id: u16) -> Option<&'static str> {
    SynthKind::ALL
        .iter()
        .copied()
        .find(|s| s.type_id() == type_id)
        .map(SynthKind::label)
}

fn parameter_name(type_id: u16, index: usize, name: &str) -> String {
    if !name.is_empty() {
        return name.to_owned();
    }
    if type_id == SynthKind::TransistorBass.type_id()
        && let Some(label) = [
            "Distortion",
            "Drive HP",
            "Drive",
            "Drive Vol.",
            "PW",
            "LFO Speed",
            "LFO to PW",
        ]
        .get(index.saturating_sub(7))
        .filter(|_| index >= 7)
    {
        return (*label).to_owned();
    }
    format!("Parameter {index}")
}

/// Validate every change before touching the song, so a bad index/value cannot leave a partial patch.
fn set_instrument_params(
    song: &mut Song,
    request: SetInstrumentParams,
) -> Result<(usize, Vec<(usize, f32)>), String> {
    if request.changes.is_empty() || request.changes.len() > 32 {
        return Err("provide 1–32 parameter changes".into());
    }
    let channel_index = song
        .channels
        .iter()
        .position(|ch| ch.id == request.channel_id && ch.id != 0)
        .ok_or_else(|| format!("channel {} is not in the open song", request.channel_id))?;
    let channel = &mut song.channels[channel_index];
    let module = channel.rack.get_mut(request.module_id).ok_or_else(|| {
        format!(
            "module {} is not on channel {}",
            request.module_id, request.channel_id
        )
    })?;
    if built_in_instrument(module.type_id).is_none() {
        return Err("module is not a built-in instrument".into());
    }
    let mut seen = std::collections::HashSet::new();
    let mut changes = Vec::with_capacity(request.changes.len());
    for change in request.changes {
        if !seen.insert(change.index)
            || module.param_spec(change.index).is_none()
            || module.params.get(change.index).is_none()
            || !change.value.is_finite()
            || !(0.0..=1.0).contains(&change.value)
        {
            return Err(format!(
                "invalid or duplicate parameter {}: use a unique index and a normalized value from 0 to 1",
                change.index
            ));
        }
        changes.push((change.index, change.value as f32));
    }
    for &(index, value) in &changes {
        module.params[index] = value;
        module.init_follows(index as i32);
    }
    Ok((channel_index, changes))
}

fn synth(label: &str) -> Option<ChannelKind> {
    if label.eq_ignore_ascii_case("Drums") {
        return Some(ChannelKind::Drums);
    }
    SynthKind::ALL
        .iter()
        .copied()
        .find(|s| s.label().eq_ignore_ascii_case(label))
        .map(ChannelKind::Synth)
}

/// Mutates only a clone. If any track is invalid, the caller's song stays untouched.
fn arrange(song: &Song, request: Arrangement) -> Result<(Song, Vec<Value>), String> {
    if request.tracks.is_empty() || request.tracks.len() > 16 {
        return Err("provide 1–16 tracks".into());
    }
    let mut result = song.clone();
    if let Some(bpm) = request.tempo_bpm {
        if !bpm.is_finite() || !(1.0..=999.0).contains(&bpm) {
            return Err("tempo_bpm must be 1–999".into());
        }
        result.set_tempo(bpm);
    }
    let bar_beats = f64::from(song.time_sig.bar_beats());
    let mut made = Vec::new();
    let mut event_count = 0usize;
    for pattern in request.tracks {
        if pattern.bars == 0 || pattern.bars > 16 || pattern.start_bar > 4095 {
            return Err("bars must be 1–16 and start_bar at most 4095".into());
        }
        if pattern.notes.len() > 512 {
            return Err("at most 512 notes per track".into());
        }
        event_count += pattern.notes.len();
        if event_count > 2048 {
            return Err("at most 2048 notes per arrangement".into());
        }
        let (index, replace) = match pattern.target {
            Target::Existing {
                channel_id,
                replace_clip_id,
            } => {
                let index = result
                    .channels
                    .iter()
                    .position(|ch| ch.id == channel_id)
                    .ok_or_else(|| format!("channel {channel_id} is not in the open song"))?;
                (index, replace_clip_id)
            }
            Target::New { instrument, name } => {
                let kind = synth(&instrument)
                    .ok_or_else(|| format!("unknown built-in instrument: {instrument}"))?;
                let index = result
                    .insert_channel(result.channels.len(), kind)
                    .ok_or("channel limit reached")?;
                if let Some(name) = name {
                    if name.is_empty() || name.chars().count() > 80 {
                        return Err("track name must be 1–80 characters".into());
                    }
                    result.channels[index].name = name;
                }
                (index, None)
            }
        };
        let channel = &mut result.channels[index];
        let lane = channel.lanes.first_mut().ok_or("channel has no lane")?;
        if !matches!(lane.kind, LaneKind::Notes | LaneKind::Drums) {
            return Err(format!(
                "channel {} is not a note or drum track",
                channel.id
            ));
        }
        let start = f64::from(pattern.start_bar) * bar_beats * f64::from(PPQ);
        let start_tick = u32::try_from(start as u64).map_err(|_| "clip start out of range")?;
        let length = f64::from(pattern.bars) * bar_beats;
        let max_ticks = length * f64::from(PPQ);
        let mut events = Vec::with_capacity(pattern.notes.len());
        for note in pattern.notes {
            if !note.beat.is_finite()
                || !note.length_beats.is_finite()
                || note.beat < 0.0
                || note.length_beats <= 0.0
                || (note.beat + note.length_beats) * f64::from(PPQ) > max_ticks
                || note.velocity == 0
            {
                return Err(
                    "note beat/length must fit in the clip and velocity must be 1–255".into(),
                );
            }
            let pos = (note.beat * f64::from(PPQ)).round() as u32;
            let event = if lane.kind == LaneKind::Drums {
                if channel
                    .drum_kit
                    .as_ref()
                    .is_none_or(|kit| kit.pad_note(note.key as usize).is_none())
                {
                    return Err(format!(
                        "drum pad {} is not present on channel {}",
                        note.key, channel.id
                    ));
                }
                Event::Drum {
                    pos,
                    len_beats: note.length_beats,
                    pad: note.key,
                    velocity: note.velocity,
                    pan: Event::DEFAULT_PAN,
                    value: Event::DEFAULT_PITCH,
                    repeat: 0,
                }
            } else {
                if note.key > 127 {
                    return Err("MIDI keys must be 0–127".into());
                }
                Event::Note {
                    pos,
                    len_beats: note.length_beats,
                    key: note.key,
                    velocity: note.velocity,
                    pan: Event::DEFAULT_PAN,
                    slide: false,
                }
            };
            events.push(event);
        }
        events.sort_by_key(Event::pos);
        let clip = if let Some(clip_id) = replace {
            let clip = lane
                .clips
                .iter_mut()
                .find(|c| c.id == clip_id)
                .ok_or_else(|| format!("clip {clip_id} is not on channel {}", channel.id))?;
            if clip.start_tick != start_tick || (clip.loop_len_beats - length).abs() > 0.001 {
                return Err("replacement clip must have the given start_bar and bars".into());
            }
            clip
        } else {
            if lane.clips.iter().any(|c| c.start_tick == start_tick) {
                return Err(format!(
                    "channel {} already has a clip at bar {}: use replace_clip_id",
                    channel.id, pattern.start_bar
                ));
            }
            lane.add_clip(start_tick, length)
        };
        clip.events = events;
        let clip_id = clip.id;
        made.push(
            json!({"channel_id": channel.id, "channel_name": channel.name, "clip_id": clip_id}),
        );
    }
    Ok((result, made))
}

impl HoneydewApp {
    /// The requests that came in, answered. Returns a `seek`'s tick, for the transport to go to this frame.
    pub(super) fn control_frame(&mut self, tick: u64) -> Option<u64> {
        let requests = self.control.as_ref().map(Bridge::drain).unwrap_or_default();
        let mut seek = None;
        for request in requests {
            let seeking = request.value.get("action").and_then(Value::as_str) == Some("seek");
            if seeking && let Ok(at) = self.control_seek(&request.value) {
                seek = Some(at);
            }
            let response = self.control_call(request.value, tick);
            let _ = request.reply.send(response);
        }
        seek
    }

    /// The requests answered while no frame runs ([`background`]), and the transport moved as a frame would: Play,
    /// Stop and a seek go to the engine at once.
    fn control_background(&mut self) {
        let tick = self.audio.poll().map_or(0, |s| s.transport_tick);
        let was_playing = self.main_screen.playing;
        let seek = self.control_frame(tick);
        let playing = self.main_screen.playing;
        if playing != was_playing {
            if playing {
                self.main_screen.play_start = tick;
            }
            self.audio.send(if playing {
                AudioCommand::Play
            } else {
                AudioCommand::Stop
            });
        }
        if let Some(at) = seek {
            self.audio.send(AudioCommand::Seek(at));
            self.main_screen.play_start = at;
            // As a frame's seek: one while playing turns recording off.
            self.main_screen.armed &= !playing;
        }
    }

    /// A `seek`'s tick: `bar` (zero-based, fractions allowed) in the song's bars.
    fn control_seek(&self, value: &Value) -> Result<u64, String> {
        let bar = value
            .get("bar")
            .and_then(Value::as_f64)
            .ok_or("bar must be a number")?;
        if !bar.is_finite() || !(0.0..=4096.0).contains(&bar) {
            return Err("bar must be 0–4096".into());
        }
        let beats = bar * f64::from(self.song.time_sig.bar_beats());
        Ok((beats * f64::from(PPQ)).round() as u64)
    }

    fn control_mixer(&self) -> Value {
        let spec = crate::params::CHANNEL_PARAMS;
        json!({"channels": self.song.channels.iter().map(|ch| {
            let p = &ch.params.0;
            let name = if ch.id == 0 { "Master" } else { ch.name.as_str() };
            json!({
                "id": ch.id,
                "name": name,
                "level": p[0], "level_text": spec[0].display_text(p[0]),
                "pan": p[1], "pan_text": spec[1].display_text(p[1]),
                "mute": p[2] >= 0.5, "solo": p[3] >= 0.5,
            })
        }).collect::<Vec<_>>()})
    }

    fn control_call(&mut self, value: Value, tick: u64) -> Value {
        let answer = match value.get("action").and_then(Value::as_str) {
            Some("song") => Ok(self.control_song()),
            Some("instruments") => Ok(json!({
                "built_in": SynthKind::ALL.iter().map(|s| s.label()).chain(["Drums"]).collect::<Vec<_>>(),
                "plugins": crate::plugins::available().into_iter()
                    .filter(|p| p.kind == crate::plugins::PluginKind::Instrument)
                    .map(|p| json!({"uid": p.uid, "name": p.name, "format": p.format}))
                    .collect::<Vec<_>>()
            })),
            Some("instrument_params") => value
                .get("channel_id")
                .and_then(Value::as_i64)
                .and_then(|id| i32::try_from(id).ok())
                .ok_or_else(|| "channel_id must be an integer".to_owned())
                .and_then(|id| self.control_instrument_params(id)),
            Some("set_instrument_params") => serde_json::from_value::<SetInstrumentParams>(
                value.get("edit").cloned().unwrap_or(Value::Null),
            )
            .map_err(|e| e.to_string())
            .and_then(|edit| {
                let module_id = edit.module_id;
                let (channel, changes) = set_instrument_params(&mut self.song, edit)?;
                for &(index, value) in &changes {
                    self.audio.send(AudioCommand::SetParam {
                        channel,
                        instance_id: module_id,
                        index: index as i32,
                        value,
                    });
                }
                self.song_edited(SongEdit::ChannelParam, tick);
                Ok(json!({"channel_id": self.song.channels[channel].id,
                            "module_id": module_id, "changed": changes.len(), "undoable": false}))
            }),
            Some("arrange") => {
                let parsed = serde_json::from_value::<Arrangement>(
                    value.get("arrangement").cloned().unwrap_or(Value::Null),
                );
                parsed.map_err(|e| e.to_string()).and_then(|arrangement| {
                    let (song, made) = arrange(&self.song, arrangement)?;
                    self.song = song;
                    self.song_edited(SongEdit::Final, tick);
                    Ok(json!({"created": made, "undo_steps": 1}))
                })
            }
            Some("play") => {
                self.main_screen.playing = true;
                Ok(json!({"playing": true}))
            }
            Some("stop") => {
                self.main_screen.playing = false;
                Ok(json!({"playing": false}))
            }
            // The transport goes there this frame ([`Self::control_frame`]).
            Some("seek") => self
                .control_seek(&value)
                .map(|tick| json!({"bar": value.get("bar"), "tick": tick})),
            Some("mixer") => Ok(self.control_mixer()),
            Some("set_mixer") => serde_json::from_value::<Vec<MixerChange>>(
                value.get("changes").cloned().unwrap_or(Value::Null),
            )
            .map_err(|e| e.to_string())
            .and_then(|changes| {
                let set = set_mixer(&mut self.song, changes)?;
                for &(channel, index, value) in &set {
                    self.audio.send(AudioCommand::SetParam {
                        channel,
                        instance_id: 0,
                        index: index as i32,
                        value,
                    });
                }
                self.song_edited(SongEdit::ChannelParam, tick);
                Ok(json!({"changed": set.len(), "undoable": false}))
            }),
            _ => Err("unknown action".into()),
        };
        match answer {
            Ok(result) => json!({"result": result}),
            Err(error) => json!({"error": error}),
        }
    }

    fn control_song(&self) -> Value {
        json!({
            "title": self.song.title,
            "tempo_bpm": self.song.tempo_bpm,
            "beats_per_bar": self.song.time_sig.bar_beats(),
            "channels": self.song.channels.iter().skip(1).map(|ch| json!({
                "id": ch.id,
                "name": ch.name,
                "lane_kind": format!("{:?}", ch.lanes[0].kind).to_lowercase(),
                "instrument": ch.rack.modules.iter().filter(|m| {
                    m.type_id == crate::song::plugin::PLUGIN_INSTRUMENT
                        || SynthKind::ALL.iter().any(|s| s.type_id() == m.type_id)
                }).map(|m| m.title()).collect::<Vec<_>>(),
                "drum_pads": ch.drum_kit.as_ref().map(|kit| kit.slots.iter()
                    .enumerate().map(|(index, slot)| json!({"index": index, "name": slot.display_name()}))
                    .collect::<Vec<_>>()),
                "clips": ch.lanes[0].clips.iter().map(|c| json!({
                    "id": c.id,
                    "start_bar": f64::from(c.start_tick) / f64::from(PPQ) / f64::from(self.song.time_sig.bar_beats()),
                    "bars": c.loop_len_beats / f64::from(self.song.time_sig.bar_beats()),
                    "note_count": c.events.len()
                })).collect::<Vec<_>>()
            })).collect::<Vec<_>>()
        })
    }

    fn control_instrument_params(&self, channel_id: i32) -> Result<Value, String> {
        let channel = self
            .song
            .channels
            .iter()
            .find(|ch| ch.id == channel_id && ch.id != 0)
            .ok_or_else(|| format!("channel {channel_id} is not in the open song"))?;
        let modules = channel.rack.modules.iter().filter_map(|module| {
            let instrument = built_in_instrument(module.type_id)?;
            let params = module.params.iter().enumerate().filter_map(|(index, &value)| {
                let spec = module.param_spec(index)?;
                let kind = match spec.kind {
                    ParamKind::Continuous { .. } => "continuous",
                    ParamKind::Menu { .. } | ParamKind::MenuBy { .. } => "menu",
                    ParamKind::Toggle => "toggle",
                };
                Some(json!({"index": index,
                    "name": parameter_name(module.type_id, index, spec.name),
                    "kind": kind, "options": spec.kind.items(),
                    "value": value, "default": spec.default,
                    "display": spec.display_text(value)}))
            }).collect::<Vec<_>>();
            Some(json!({"module_id": module.instance_id, "instrument": instrument, "parameters": params}))
        }).collect::<Vec<_>>();
        Ok(json!({"channel_id": channel_id, "channel_name": channel.name, "modules": modules}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mcp_tools_parse_and_each_maps_to_an_action() {
        let tools = tools();
        let names: Vec<&str> = tools
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            [
                "get_song",
                "list_instruments",
                "get_instrument_params",
                "set_instrument_params",
                "apply_arrangement",
                "play",
                "stop",
                "seek",
                "get_mixer",
                "set_mixer"
            ]
        );
        for name in names {
            assert!(tool_action(name, &json!({})).is_some(), "{name}");
        }
        assert!(tool_action("render", &json!({})).is_none());
    }

    #[test]
    fn transport_and_mixer_actions() {
        let mut app = HoneydewApp::default();
        app.control_call(json!({"action": "play"}), 0);
        assert!(app.main_screen.playing);
        app.control_call(json!({"action": "stop"}), 0);
        assert!(!app.main_screen.playing);
        let bar_ticks = u64::from(app.song.time_sig.bar_beats()) * u64::from(PPQ);
        assert_eq!(app.control_seek(&json!({"bar": 2.0})), Ok(2 * bar_ticks));
        assert!(app.control_seek(&json!({"bar": -1.0})).is_err());
        let bad = app.control_call(
            json!({"action": "set_mixer", "changes": [{"channel_id": 0, "level": 2.0}]}),
            0,
        );
        assert!(bad.get("error").is_some(), "{bad}");
        let ok = app.control_call(
            json!({"action": "set_mixer", "changes": [{"channel_id": 0, "level": 0.5, "mute": true}]}),
            0,
        );
        assert_eq!(ok["result"]["changed"], 2, "{ok}");
        assert_eq!(app.song.channels[0].params.0[0], 0.5);
        let mixer = app.control_call(json!({"action": "mixer"}), 0);
        assert_eq!(mixer["result"]["channels"][0]["mute"], true);
    }

    #[test]
    fn arrangement_is_one_valid_song_with_builtin_drum_and_synth() {
        let before = Song::default();
        let request: Arrangement = serde_json::from_value(json!({"tracks": [
            {"target": {"kind": "new", "instrument": "Drums", "name": "Beat"},
             "start_bar": 0, "bars": 1, "notes": [{"beat": 0.0, "length_beats": 0.25, "key": 0, "velocity": 220}]},
            {"target": {"kind": "new", "instrument": "MiniSynth", "name": "Bass"},
             "start_bar": 0, "bars": 1, "notes": [{"beat": 0.5, "length_beats": 0.5, "key": 48, "velocity": 180}]}
        ]})).unwrap();
        let (song, made) = arrange(&before, request).unwrap();
        assert_eq!(made.len(), 2);
        assert_eq!(song.channels[1].lanes[0].clips[0].events.len(), 1);
        assert_eq!(song.channels[2].lanes[0].clips[0].events.len(), 1);
        assert!(crate::flm::write(&song).is_ok());
        assert_eq!(before.channels.len(), 1);
    }

    #[test]
    fn invalid_note_does_not_change_input_song() {
        let before = Song::default();
        let request: Arrangement = serde_json::from_value(json!({"tracks": [{
            "target": {"kind": "new", "instrument": "MiniSynth"},
            "start_bar": 0, "bars": 1,
            "notes": [{"beat": 4.0, "length_beats": 1.0, "key": 60, "velocity": 100}]
        }]}))
        .unwrap();
        assert!(arrange(&before, request).is_err());
        assert_eq!(before.channels.len(), 1);
    }

    #[test]
    fn existing_plugin_sound_is_kept_when_adding_and_replacing_notes() {
        let mut before = Song::default();
        before.add_channel(ChannelKind::Instrument);
        let channel_id = before.channels[1].id;
        let module_id = before.channels[1]
            .rack
            .add(crate::song::plugin::PLUGIN_INSTRUMENT);
        let module = before.channels[1].rack.get_mut(module_id).unwrap();
        module.file.plugin = Some(Box::new(crate::song::plugin::PluginRef {
            name: "Vital".into(),
            format: "clap".into(),
            id: "vital.test".into(),
            state: vec![1, 2, 3].into(),
            ..Default::default()
        }));
        let first: Arrangement = serde_json::from_value(json!({"tracks": [{
            "target": {"kind": "existing", "channel_id": channel_id},
            "start_bar": 0, "bars": 1,
            "notes": [{"beat": 0.0, "length_beats": 1.0, "key": 60, "velocity": 200}]
        }]}))
        .unwrap();
        let (song, made) = arrange(&before, first).unwrap();
        let clip_id = made[0]["clip_id"].as_u64().unwrap();
        let second: Arrangement = serde_json::from_value(json!({"tracks": [{
            "target": {"kind": "existing", "channel_id": channel_id, "replace_clip_id": clip_id},
            "start_bar": 0, "bars": 1,
            "notes": [{"beat": 0.5, "length_beats": 0.5, "key": 48, "velocity": 180}]
        }]}))
        .unwrap();
        let (changed, _) = arrange(&song, second).unwrap();
        assert_eq!(changed.channels[1].lanes[0].clips.len(), 1);
        assert_eq!(changed.channels[1].rack, before.channels[1].rack);
        assert_eq!(
            changed.channels[1]
                .rack
                .get(module_id)
                .unwrap()
                .file
                .plugin
                .as_ref()
                .unwrap()
                .state
                .as_ref(),
            &[1, 2, 3]
        );
    }

    #[test]
    fn control_action_updates_open_song_through_edit_path() {
        let mut app = HoneydewApp::default();
        let answer = app.control_call(
            json!({"action": "arrange", "arrangement": {
                "tempo_bpm": 138.0,
                "tracks": [{"target": {"kind": "new", "instrument": "Drums"},
                    "start_bar": 0, "bars": 1,
                    "notes": [{"beat": 0.0, "length_beats": 0.25, "key": 0, "velocity": 220}] }]
            }}),
            0,
        );
        assert!(answer.get("error").is_none(), "{answer}");
        assert_eq!(app.song.tempo_bpm, 138.0);
        assert_eq!(app.song.channels.len(), 2);
        assert_eq!(app.song.channels[1].lanes[0].clips.len(), 1);
        let undone = app
            .history
            .undo()
            .expect("one control call is one undo step");
        assert_eq!(undone.channels.len(), 1);
        assert_eq!(undone.tempo_bpm, 120.0);
        assert!(app.history.undo().is_none());
    }

    #[test]
    fn built_in_parameters_are_read_and_applied_atomically() {
        let mut app = HoneydewApp::default();
        let channel = app
            .song
            .insert_channel(1, ChannelKind::Synth(SynthKind::TransistorBass))
            .unwrap();
        let channel_id = app.song.channels[channel].id;
        let module_id = app.song.channels[channel].rack.modules[0].instance_id;
        let found = app.control_call(
            json!({"action": "instrument_params", "channel_id": channel_id}),
            0,
        );
        let params = &found["result"]["modules"][0]["parameters"];
        assert_eq!(found["result"]["modules"][0]["module_id"], module_id);
        assert_eq!(params[9]["name"], "Drive");
        let bad = app.control_call(
            json!({"action": "set_instrument_params", "edit": {
                "channel_id": channel_id, "module_id": module_id,
                "changes": [{"index": 2, "value": 0.45}, {"index": 99, "value": 0.5}]
            }}),
            0,
        );
        assert!(bad.get("error").is_some(), "{bad}");
        assert_eq!(app.song.channels[channel].rack.modules[0].params[2], 1.0);
        let changed = app.control_call(
            json!({"action": "set_instrument_params", "edit": {
                "channel_id": channel_id, "module_id": module_id,
                "changes": [{"index": 2, "value": 0.45}, {"index": 9, "value": 0.8}]
            }}),
            0,
        );
        assert_eq!(changed["result"]["changed"], 2, "{changed}");
        assert_eq!(app.song.channels[channel].rack.modules[0].params[2], 0.45);
        assert_eq!(app.song.channels[channel].rack.modules[0].params[9], 0.8);
    }

    #[test]
    fn external_plugin_module_cannot_be_edited_as_built_in() {
        let mut song = Song::default();
        let index = song.insert_channel(1, ChannelKind::Instrument).unwrap();
        let module_id = song.channels[index]
            .rack
            .add(crate::song::plugin::PLUGIN_INSTRUMENT);
        let request: SetInstrumentParams = serde_json::from_value(json!({
            "channel_id": song.channels[index].id, "module_id": module_id,
            "changes": [{"index": 0, "value": 0.0}]
        }))
        .unwrap();
        assert!(set_instrument_params(&mut song, request).is_err());
    }
}
