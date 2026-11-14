//! Opt-in local inspector for headless screenshot / JS access to the app's
//! WKWebView (project-management#376).
//!
//! Gate: `STRIKEHUB_INSPECTOR=1` (or `true`). Default OFF — when the variable
//! is unset this module does nothing and adds zero behavior.
//!
//! Design (macOS only):
//!
//! - A unix-domain socket (`$TMPDIR/strikehub-inspect-<pid>.sock`, mode 0600)
//!   is bound at startup before the window exists; the socket path is printed
//!   on stdout and also written to `$TMPDIR/strikehub-inspect-<pid>.path`.
//! - Requests are newline-delimited JSON: `{"cmd":"screenshot"}`,
//!   `{"cmd":"eval","js":"..."}`, `{"cmd":"click","css":"..."}`,
//!   `{"cmd":"type","css":"...","text":"..."}`,
//!   `{"cmd":"wait","css":"...","timeout_ms":N}`, `{"cmd":"text","css":"..."}`,
//!   `{"cmd":"targets"}`.
//! - Responses are single-line JSON: `{"ok":true,...}` or `{"ok":false,"error":"..."}`.
//! - Each accepted connection is handled on its own thread, so one
//!   in-flight (possibly slow) command cannot block the accept loop or any
//!   other connection (issue #140).
//! - Main-thread hops are bounded by `policy::MAIN_THREAD_HOP_TIMEOUT` (10s);
//!   a command that misses the bound gets a
//!   `{"ok":false,"error":"main-thread busy"}` response instead of a silent
//!   stall.
//! - takeSnapshot results are retained only in a bounded cache (one slot
//!   per webview, at most 3 slots, least-recently-used evicted first; slots
//!   of freed webviews are dropped — issue #139), so a long inspector
//!   session cannot accumulate them.
//!
//! Only PUBLIC WKWebView APIs are used (`takeSnapshotWithConfiguration:`,
//! `evaluateJavaScript:completionHandler:`, `title`, `URL`); the WKWebView is
//! located by walking NSApplication's window/view hierarchy for a view whose
//! class name is `WKWebView`. All AppKit/WebKit calls hop onto the main
//! queue (GCD) and results come back over an mpsc channel.
//!
//! Companion CLI: `tools/strikehub-inspect.py`.
//!
//! Mac verification (objc/`msg_send!` correctness only surfaces at runtime,
//! and the cfg(macos) code below is the one part of the app no
//! Linux/Windows CI job compiles — run on macOS after any change to this
//! module):
//!
//! ```text
//! STRIKEHUB_INSPECTOR=1 cargo run              # prints STRIKEHUB_INSPECTOR_SOCKET=...
//! python3 tools/strikehub-inspect.py eval ''   # {"cmd":"eval","js":""}
//! python3 tools/strikehub-inspect.py eval "'é'" # {"cmd":"eval","js":"'é'"}
//! ```
//!
//! Expected: the empty-`js` eval returns `{"ok":true,"result":"null"}` (JS
//! `undefined`) and the single-quoted `é` eval returns `{"ok":true,"result":"é"}`
//! — together they prove zero-length and non-ASCII (UTF-8) JS round-trip
//! through `NSString` (the edges jtomek's macOS probe found, #132 review).

#[cfg(target_os = "macos")]
pub fn maybe_start() {
    let enabled = std::env::var("STRIKEHUB_INSPECTOR")
        .ok()
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    if !enabled {
        return;
    }
    match server::start() {
        Ok(socket_path) => {
            println!("STRIKEHUB_INSPECTOR_SOCKET={}", socket_path.display());
        }
        Err(e) => {
            eprintln!("strikehub-inspect: failed to start inspector: {e}");
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn maybe_start() {
    if let Ok(v) = std::env::var("STRIKEHUB_INSPECTOR")
        && (v == "1" || v.eq_ignore_ascii_case("true"))
    {
        eprintln!("strikehub-inspect: STRIKEHUB_INSPECTOR is only supported on macOS; ignored");
    }
}

/// Platform-independent inspector decisions (issue #140).
///
/// The GCD-backed socket server below is macOS-only and cannot compile on
/// Linux. These values decide how the server *behaves* — the main-thread hop
/// bound and the shape of the busy response it emits when that bound is hit
/// — so they live outside the `#[cfg(target_os = "macos")]` plumbing and are
/// unit-testable on the Linux CI gate.
pub mod policy {
    use std::time::Duration;

    /// How long a command waits for its hop onto the macOS main thread (GCD)
    /// before giving up and reporting the main thread as busy.
    ///
    /// All inspector clients share the one serialized main-thread hop, so
    /// with the old 60s bound a single slow `eval` could starve every other
    /// client for a full minute (issue #140). 10s caps the worst case; the
    /// timed-out command gets a structured "main-thread busy" response
    /// instead of a silent stall.
    pub const MAIN_THREAD_HOP_TIMEOUT: Duration = Duration::from_secs(10);

    /// How long an accepted connection may sit idle before the server drops
    /// it.
    pub const CONN_IDLE_TIMEOUT: Duration = Duration::from_secs(300);

    /// Error string reported when a command's main-thread hop does not
    /// complete within [`MAIN_THREAD_HOP_TIMEOUT`].
    pub const MAIN_THREAD_BUSY_ERROR: &str = "main-thread busy";

    /// The single-line JSON response returned when a main-thread hop times
    /// out: `{"ok":false,"error":"main-thread busy"}`.
    pub fn main_thread_busy_response() -> serde_json::Value {
        serde_json::json!({ "ok": false, "error": MAIN_THREAD_BUSY_ERROR })

// ------------------------------------------------------------------
// Bounded snapshot result cache (pure, platform-independent)
// ------------------------------------------------------------------
//
// Issue #139: takeSnapshot results must not accumulate without bound over
// a long inspector session. This module is the single retention point for
// them: one slot per webview (a new snapshot replaces that webview's slot),
// at most `MAX_ENTRIES` (3) slots across webviews, least-recently-used
// evicted first, and slots of freed webviews dropped.
//
// It is deliberately pure (no AppKit, no threads) and lives OUTSIDE the
// macOS-only `server` module below, so it compiles — and is unit-tested —
// on every target. The CI gates run on Linux, where the macOS path never
// compiles; the bound-keeping logic is what is testable there.
#[allow(dead_code)] // The API surface is wider than the (macOS-only) usage;
// the rest is exercised by the (every-target) unit tests below.
mod snapshot_cache {
    use std::collections::VecDeque;

    /// Hard cap on the number of webview slots retained at once.
    pub const MAX_ENTRIES: usize = 3;

    /// One retained takeSnapshot result.
    #[derive(Debug)]
    struct Slot {
        /// Opaque webview id (the WKWebView pointer on macOS).
        key: u64,
        /// The encoded result (PNG base64 on macOS).
        data: String,
    }

    /// Bounded LRU store: one slot per webview, at most [`MAX_ENTRIES`].
    #[derive(Debug)]
    pub struct SnapshotCache {
        /// Front = most recently used, back = evicted first.
        slots: VecDeque<Slot>,
    }

    impl SnapshotCache {
        pub const fn new() -> Self {
            Self {
                slots: VecDeque::new(),
            }
        }

        /// Store a fresh result for `key`.
        ///
        /// Re-storing for a webview that already has a slot replaces that
        /// slot in place (single slot per webview) and marks it most
        /// recently used — no eviction. If inserting a *new* webview would
        /// exceed [`MAX_ENTRIES`], the least-recently-used slot is evicted
        /// and its data returned.
        pub fn insert(&mut self, key: u64, data: String) -> Option<String> {
            if let Some(pos) = self.slots.iter().position(|s| s.key == key) {
                let mut slot = self.slots.remove(pos).unwrap();
                slot.data = data;
                self.slots.push_front(slot);
                return None;
            }
            let evicted = if self.slots.len() >= MAX_ENTRIES {
                self.slots.pop_back().map(|s| s.data)
            } else {
                None
            };
            self.slots.push_front(Slot { key, data });
            evicted
        }

        /// The most recent stored result for `key`, if any. Reading a slot
        /// refreshes its recency, so an in-use webview's snapshot is not
        /// the first to be evicted.
        pub fn get(&mut self, key: u64) -> Option<&String> {
            let pos = self.slots.iter().position(|s| s.key == key)?;
            let slot = self.slots.remove(pos).unwrap();
            self.slots.push_front(slot);
            self.slots.front().map(|s| &s.data)
        }

        /// Drop the slot for `key` (its webview was freed). Returns the
        /// stored data if a slot existed.
        pub fn remove(&mut self, key: u64) -> Option<String> {
            let pos = self.slots.iter().position(|s| s.key == key)?;
            Some(self.slots.remove(pos).unwrap().data)
        }

        /// Drop every slot whose webview is no longer live. Returns the
        /// number of slots dropped.
        pub fn retain_live(&mut self, is_live: impl Fn(u64) -> bool) -> usize {
            let before = self.slots.len();
            self.slots.retain(|s| is_live(s.key));
            before - self.slots.len()
        }

        pub fn len(&self) -> usize {
            self.slots.len()
        }

        pub fn is_empty(&self) -> bool {
            self.slots.is_empty()
        }

        /// Webview keys in MRU order (most recent first).
        pub fn keys_mru(&self) -> impl Iterator<Item = u64> + '_ {
            self.slots.iter().map(|s| s.key)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn hop_timeout_is_capped_at_10_seconds() {
            // #140: the old 60s bound let one slow eval starve every other
            // inspector client; the bound must be 10s (well under 60s).
            assert_eq!(MAIN_THREAD_HOP_TIMEOUT, Duration::from_secs(10));
            assert!(MAIN_THREAD_HOP_TIMEOUT < Duration::from_secs(60));
        }

        #[test]
        fn conn_idle_timeout_is_5_minutes() {
            assert_eq!(CONN_IDLE_TIMEOUT, Duration::from_secs(300));
        }

        #[test]
        fn busy_response_is_ok_false_with_busy_error() {
            let v = main_thread_busy_response();
            assert_eq!(v.get("ok"), Some(&serde_json::json!(false)));
            assert_eq!(
                v.get("error"),
                Some(&serde_json::json!(MAIN_THREAD_BUSY_ERROR))
            );
            // Exactly the two documented keys, nothing else.
            assert_eq!(v.as_object().map(|o| o.len()), Some(2));
        }

        #[test]
        fn busy_response_stays_single_line_json() {
            // Inspector responses are newline-delimited; the busy response
            // must fit on one line so framing stays intact.
            let s = serde_json::to_string(&main_thread_busy_response()).unwrap();
            assert!(!s.contains('\n'));
            let reparsed: serde_json::Value = serde_json::from_str(&s).unwrap();
            assert_eq!(reparsed.get("ok"), Some(&serde_json::json!(false)));
            assert_eq!(
                reparsed.get("error"),
                Some(&serde_json::json!("main-thread busy"))
            );

        /// A cache holding `data-<key>` for each key, inserted in order
        /// (so the last key is the MRU).
        fn cache_with(keys: &[u64]) -> SnapshotCache {
            let mut c = SnapshotCache::new();
            for k in keys {
                assert!(c.insert(*k, format!("data-{k}")).is_none());
            }
            c
        }

        #[test]
        fn fresh_cache_is_empty() {
            let c = SnapshotCache::new();
            assert!(c.is_empty());
            assert_eq!(c.len(), 0);
        }

        #[test]
        fn cap_is_respected_and_lru_is_evicted_first() {
            let mut c = SnapshotCache::new();
            c.insert(1, "d1".into());
            c.insert(2, "d2".into());
            assert_eq!(c.len(), 2);
            // Third entry still fits.
            assert_eq!(c.insert(3, "d3".into()), None);
            assert_eq!(c.len(), MAX_ENTRIES);
            // Fourth entry overflows: the least-recently-used (1) goes.
            assert_eq!(c.insert(4, "d4".into()), Some("d1".to_string()));
            assert_eq!(c.len(), MAX_ENTRIES);
            assert!(c.get(1).is_none());
            assert_eq!(c.get(4), Some(&"d4".to_string()));
            assert!(c.get(2).is_some());
            assert!(c.get(3).is_some());
            assert_eq!(c.keys_mru().count(), MAX_ENTRIES);
        }

        #[test]
        fn reads_refresh_recency_and_change_eviction_order() {
            let mut c = cache_with(&[1, 2, 3]); // MRU order: 3, 2, 1
            // Reading 1 makes it the MRU, so 2 becomes the LRU.
            assert_eq!(c.get(1), Some(&"data-1".to_string()));
            let evicted = c.insert(4, "data-4".into());
            assert_eq!(evicted, Some("data-2".to_string()));
            assert!(c.get(1).is_some());
            assert!(c.get(3).is_some());
            assert!(c.get(4).is_some());
            assert!(c.get(2).is_none());
        }

        #[test]
        fn one_slot_per_webview_replaces_in_place() {
            let mut c = SnapshotCache::new();
            c.insert(7, "old".into());
            c.insert(7, "new".into());
            assert_eq!(c.len(), 1);
            assert_eq!(c.get(7), Some(&"new".to_string()));
            // Re-storing a known webview never evicts, even at the cap,
            // and refreshes its recency.
            let mut full = cache_with(&[1, 2, 3]);
            assert_eq!(full.insert(2, "again".into()), None);
            assert_eq!(full.len(), MAX_ENTRIES);
            assert_eq!(full.get(2), Some(&"again".to_string()));
            assert_eq!(
                full.keys_mru().collect::<Vec<_>>(),
                vec![2, 3, 1] // 2 re-stored => MRU
            );
        }

        #[test]
        fn remove_drops_a_freed_webview_slot() {
            let mut c = cache_with(&[1, 2]);
            assert_eq!(c.remove(1), Some("data-1".to_string()));
            assert_eq!(c.len(), 1);
            assert!(c.get(1).is_none());
            // Removing an unknown key is a no-op.
            assert_eq!(c.remove(99), None);
            assert_eq!(c.len(), 1);
        }

        #[test]
        fn retain_live_drops_only_freed_webviews() {
            let mut c = cache_with(&[1, 2, 3]);
            let dropped = c.retain_live(|k| k != 2);
            assert_eq!(dropped, 1);
            assert_eq!(c.len(), 2);
            assert!(c.get(1).is_some());
            assert!(c.get(2).is_none());
            assert!(c.get(3).is_some());
            // Nothing live to drop.
            assert_eq!(c.retain_live(|_| true), 0);
        }

        #[test]
        fn cap_holds_under_mixed_workload() {
            let mut c = SnapshotCache::new();
            // Interleave fresh inserts, re-stores, reads and frees; the
            // cap must hold at every step.
            for round in 0..50u64 {
                c.insert(round % 7, format!("r{round}"));
                if round % 3 == 0 {
                    c.get(round % 7);
                }
                if round % 5 == 0 {
                    c.remove((round + 1) % 7);
                }
                assert!(c.len() <= MAX_ENTRIES, "cap violated at round {round}");
            }
        }
    }
}

#[cfg(target_os = "macos")]
mod server {
    use std::io::{BufRead, Write};
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::PathBuf;
    use std::sync::mpsc;
    use std::time::Duration;

    use super::policy;
    use base64::Engine;
    use block2::{Block, RcBlock};
    use dispatch::Queue;
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send};

    type Resp = Result<serde_json::Value, String>;
    type ObjCFn = dyn Fn(*mut AnyObject, *mut AnyObject);
    type ObjCBlock2 = Block<ObjCFn>;

    /// Bounded retention for takeSnapshot results (issue #139): one slot
    /// per webview, at most `MAX_ENTRIES` (3) slots, LRU; slots of freed
    /// webviews are dropped on the next screenshot request.
    static SNAPSHOT_CACHE: std::sync::Mutex<super::snapshot_cache::SnapshotCache> =
        std::sync::Mutex::new(super::snapshot_cache::SnapshotCache::new());

    // ------------------------------------------------------------------
    // Socket server
    // ------------------------------------------------------------------

    pub fn start() -> anyhow::Result<PathBuf> {
        let tmp = std::env::temp_dir();
        let pid = std::process::id();
        let socket_path = tmp.join(format!("strikehub-inspect-{pid}.sock"));
        let path_file = tmp.join(format!("strikehub-inspect-{pid}.path"));

        let _ = std::fs::remove_file(&socket_path);
        let listener = UnixListener::bind(&socket_path)?;
        std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600))?;
        std::fs::write(&path_file, socket_path.to_string_lossy().as_bytes())?;
        std::fs::set_permissions(&path_file, std::fs::Permissions::from_mode(0o600))?;

        std::thread::Builder::new()
            .name("insp-accept".into())
            .spawn(move || {
                for stream in listener.incoming() {
                    if let Ok(stream) = stream {
                        // One thread per accepted connection (issue #140):
                        // the acceptor must never block on an in-flight
                        // command — a slow `eval` on one client used to hold
                        // the sequential accept loop and starve every
                        // subsequent client.
                        std::thread::Builder::new()
                            .name("insp-conn".into())
                            .spawn(move || handle_conn(stream))
                            .ok();
                    }
                }
            })?;

        Ok(socket_path)
    }

    fn handle_conn(stream: UnixStream) {
        let _ = stream.set_read_timeout(Some(policy::CONN_IDLE_TIMEOUT));
        let reader = match stream.try_clone() {
            Ok(r) => r,
            Err(_) => return,
        };
        let mut writer = stream;
        for line in std::io::BufReader::new(reader).lines() {
            let Ok(line) = line else { break };
            let line = line.trim().to_string();
            if line.is_empty() {
                continue;
            }
            let resp = dispatch_cmd(&line);
            let mut payload = serde_json::to_string(&resp).unwrap_or_else(|_| {
                r#"{"ok":false,"error":"internal: response encode failed"}"#.into()
            });
            payload.push('\n');
            if writer.write_all(payload.as_bytes()).is_err() || writer.flush().is_err() {
                break;
            }
        }
    }

    fn dispatch_cmd(request: &str) -> serde_json::Value {
        let req: serde_json::Value = match serde_json::from_str(request) {
            Ok(v) => v,
            Err(e) => return serde_json::json!({ "ok": false, "error": format!("bad json: {e}") }),
        };
        let cmd = req.get("cmd").and_then(|v| v.as_str()).unwrap_or("");
        match cmd {
            "screenshot" => cmd_screenshot(),
            "eval" => cmd_eval(req.get("js").and_then(|v| v.as_str()).unwrap_or("")),
            "click" => cmd_eval(&js_shim_click(
                req.get("css").and_then(|v| v.as_str()).unwrap_or(""),
            )),
            "type" => cmd_eval(&js_shim_type(
                req.get("css").and_then(|v| v.as_str()).unwrap_or(""),
                req.get("text").and_then(|v| v.as_str()).unwrap_or(""),
            )),
            "wait" => cmd_wait(
                req.get("css").and_then(|v| v.as_str()).unwrap_or(""),
                req.get("timeout_ms")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(15_000),
            ),
            "text" => cmd_eval(&js_shim_text(
                req.get("css").and_then(|v| v.as_str()).unwrap_or(""),
            )),
            "targets" => cmd_targets(),
            other => serde_json::json!({
                "ok": false,
                "error": format!("unknown cmd: {other} (expected screenshot|eval|click|type|wait|text|targets)")
            }),
        }
    }

    // ------------------------------------------------------------------
    // Commands
    // ------------------------------------------------------------------

    fn cmd_screenshot() -> serde_json::Value {
        let (tx, rx) = mpsc::channel::<Resp>();
        Queue::main().exec_async(move || {
            // Issue #139: walk the hierarchy once; the same list reaps the
            // cache entries of webviews that were freed since the last
            // snapshot and yields this request's webview.
            let live = live_wkwebviews();
            reap_freed_webview_entries(&live);
            let Some(wv_ptr) = live.into_iter().next() else {
                let _ = tx.send(Err("no WKWebView found (window not up yet?)".into()));
                return;
            };
            let wv: &AnyObject = unsafe { &*wv_ptr };
            let wv_key = wv_ptr as usize as u64;
            let block: RcBlock<ObjCFn> =
                RcBlock::new(move |image: *mut AnyObject, err: *mut AnyObject| {
                    if image.is_null() {
                        let _ = tx.send(Err(format!(
                            "takeSnapshot failed: {}",
                            obj_c_error_message(err)
                        )));
                        return;
                    }
                    match image_to_png_base64(image) {
                        Ok(b64) => {
                            // Issue #139: the result is retained in the
                            // bounded per-webview cache and the requester is
                            // served from that stored slot — a long session
                            // holds at most MAX_ENTRIES snapshots.
                            let png_b64 = {
                                let mut cache =
                                    SNAPSHOT_CACHE.lock().unwrap_or_else(|p| p.into_inner());
                                let _evicted = cache.insert(wv_key, b64);
                                cache.get(wv_key).map(String::clone)
                            };
                            match png_b64 {
                                Some(png_b64) => {
                                    let _ = tx.send(Ok(
                                        serde_json::json!({ "ok": true, "png_b64": png_b64 }),
                                    ));
                                }
                                // Practically unreachable (a concurrent
                                // request would have to evict this slot
                                // between insert and get); fail loudly
                                // rather than drop the screenshot.
                                None => {
                                    let _ = tx.send(Err(
                                        "internal: snapshot slot evicted before serve".into(),
                                    ));
                                }
                            }
                        }
                        Err(e) => {
                            let _ = tx.send(Err(format!("png encode failed: {e}")));
                        }
                    }
                });
            let cfg: *const AnyObject = unsafe { msg_send![class!(WKSnapshotConfiguration), new] };
            unsafe {
                let _: () = msg_send![wv,
                    takeSnapshotWithConfiguration: cfg,
                    completionHandler: (&*block) as *const ObjCBlock2
                ];
            }
        });
        recv_or_timeout(rx)
    }

    fn cmd_eval(js: &str) -> serde_json::Value {
        let js = js.to_string();
        let (tx, rx) = mpsc::channel::<Resp>();
        Queue::main().exec_async(move || {
            let Some(wv_ptr) = find_wkwebview() else {
                let _ = tx.send(Err("no WKWebView found (window not up yet?)".into()));
                return;
            };
            let wv: &AnyObject = unsafe { &*wv_ptr };
            let block: RcBlock<ObjCFn> =
                RcBlock::new(move |result: *mut AnyObject, err: *mut AnyObject| {
                    if !err.is_null() {
                        let _ = tx.send(Err(obj_c_error_message(err)));
                        return;
                    }
                    let _ = tx.send(Ok(serde_json::json!({
                        "ok": true,
                        "result": js_value_to_string(result),
                    })));
                });
            let js_ns = nsstring(&js);
            unsafe {
                let _: () = msg_send![wv,
                    evaluateJavaScript: &*js_ns,
                    completionHandler: (&*block) as *const ObjCBlock2
                ];
            }
            // js_ns (a +1 Retained) drops here; WebKit retains what it needs
            // for the async completion before the call returns.
            drop(js_ns);
        });
        recv_or_timeout(rx)
    }

    fn cmd_wait(css: &str, timeout_ms: u64) -> serde_json::Value {
        let probe = js_shim_exists(css);
        let deadline = std::time::Instant::now() + Duration::from_millis(timeout_ms);
        loop {
            let resp = cmd_eval(&probe);
            if resp.get("ok").and_then(|v| v.as_bool()) == Some(true) {
                let found = resp
                    .get("result")
                    .and_then(|v| v.as_str())
                    .map(|s| s == "true")
                    .unwrap_or(false);
                if found {
                    return serde_json::json!({ "ok": true });
                }
            }
            if std::time::Instant::now() >= deadline {
                return serde_json::json!({
                    "ok": false,
                    "error": format!("timeout after {timeout_ms} ms waiting for {css:?}")
                });
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    fn cmd_targets() -> serde_json::Value {
        let (tx, rx) = mpsc::channel::<Resp>();
        Queue::main().exec_async(move || {
            let Some(wv_ptr) = find_wkwebview() else {
                let _ = tx.send(Err("no WKWebView found (window not up yet?)".into()));
                return;
            };
            let wv: &AnyObject = unsafe { &*wv_ptr };
            let title = unsafe { nsstring_to_string(msg_send![wv, title]) };
            let url_obj: *const AnyObject = unsafe { msg_send![wv, URL] };
            let url = if url_obj.is_null() {
                None
            } else {
                Some(unsafe { nsstring_to_string(msg_send![url_obj, absoluteString]) })
            };
            let _ = tx.send(Ok(serde_json::json!({
                "ok": true,
                "targets": [{ "title": title, "url": url }],
            })));
        });
        recv_or_timeout(rx)
    }

    // ------------------------------------------------------------------
    // JS shims
    // ------------------------------------------------------------------

    fn js_shim_click(css: &str) -> String {
        let sel = serde_json::to_string(css).unwrap_or_else(|_| "\"\"".into());
        format!(
            "(function(){{var el=document.querySelector({sel});\
             if(!el){{throw new Error('not found: '+{sel});}}\
             el.click();return 'clicked';}})()"
        )
    }

    fn js_shim_type(css: &str, text: &str) -> String {
        let sel = serde_json::to_string(css).unwrap_or_else(|_| "\"\"".into());
        let txt = serde_json::to_string(text).unwrap_or_else(|_| "\"\"".into());
        format!(
            "(function(){{var el=document.querySelector({sel});\
             if(!el){{throw new Error('not found: '+{sel});}}\
             el.focus();el.value={txt};\
             el.dispatchEvent(new Event('input',{{bubbles:true}}));\
             el.dispatchEvent(new Event('change',{{bubbles:true}}));\
             return el.value;}})()"
        )
    }

    fn js_shim_text(css: &str) -> String {
        let sel = serde_json::to_string(css).unwrap_or_else(|_| "\"\"".into());
        format!(
            "(function(){{var el=document.querySelector({sel});\
             if(!el){{throw new Error('not found: '+{sel});}}\
             return el.innerText;}})()"
        )
    }

    fn js_shim_exists(css: &str) -> String {
        let sel = serde_json::to_string(css).unwrap_or_else(|_| "\"\"".into());
        format!("(function(){{return !!document.querySelector({sel});}})()")
    }

    // ------------------------------------------------------------------
    // Helpers
    // ------------------------------------------------------------------

    fn recv_or_timeout(rx: mpsc::Receiver<Resp>) -> serde_json::Value {
        match rx.recv_timeout(policy::MAIN_THREAD_HOP_TIMEOUT) {
            Ok(Ok(v)) => v,
            Ok(Err(e)) => serde_json::json!({ "ok": false, "error": e }),
            // #140: bounded hop. The main thread is busy with something else
            // (or a previous slow command); report it instead of stalling.
            Err(_) => policy::main_thread_busy_response(),
        }
    }

    fn nsstring(s: &str) -> objc2::rc::Retained<objc2_foundation::NSString> {
        // `stringWithUTF8String:` requires a NUL-terminated C string, but a
        // Rust `str` has no guaranteed NUL terminator — the ObjC runtime
        // scans past the buffer (heap over-read / segfault). Hand the bytes
        // to objc2-foundation's `NSString::from_str` instead of a hand-
        // rolled msg_send!: it does alloc + initWithBytes:length:encoding:
        // with the real NSUTF8StringEncoding (4), keeps the +1 retain in a
        // Retained (dropped right after evaluateJavaScript, which copies
        // what it needs), and avoids both bugs jtomek's macOS probe found
        // in 93153dc (class-send without alloc; kCFStringEncodingUTF8
        // mistaken for NSUTF8StringEncoding mangling non-ASCII).
        objc2_foundation::NSString::from_str(s)
    }

    fn nsstring_to_string(obj: *const AnyObject) -> String {
        if obj.is_null() {
            return String::new();
        }
        unsafe {
            let c: *const std::os::raw::c_char = msg_send![obj, UTF8String];
            if c.is_null() {
                return String::new();
            }
            std::ffi::CStr::from_ptr(c).to_string_lossy().into_owned()
        }
    }

    /// Convert an eval result (NSString / NSNumber / NSNull / ...) to a
    /// displayable string; JS `undefined` arrives as nil.
    fn js_value_to_string(obj: *mut AnyObject) -> String {
        if obj.is_null() {
            return "null".to_string();
        }
        unsafe {
            let cls: *const AnyObject = msg_send![obj, className];
            let cls_name = nsstring_to_string(cls);
            if cls_name == "NSNull" {
                "null".to_string()
            } else if cls_name == "NSString" {
                nsstring_to_string(obj as *const AnyObject)
            } else {
                let desc: *const AnyObject = msg_send![obj, description];
                let d = nsstring_to_string(desc);
                // JS booleans bridge as NSNumber(__NSCFBoolean): "1"/"0".
                if cls_name == "NSNumber"
                    || cls_name == "__NSCFBoolean"
                    || cls_name == "NSDecimalNumber"
                {
                    match d.as_str() {
                        "1" => return "true".to_string(),
                        "0" => return "false".to_string(),
                        _ => return d,
                    }
                }
                d
            }
        }
    }

    fn obj_c_error_message(err: *mut AnyObject) -> String {
        if err.is_null() {
            return "unknown error".to_string();
        }
        unsafe {
            let desc: *const AnyObject = msg_send![err, localizedDescription];
            let s = nsstring_to_string(desc);
            if s.is_empty() {
                "unknown error".to_string()
            } else {
                s
            }
        }
    }

    /// NSImage → PNG bytes → base64 (all public AppKit API).
    fn image_to_png_base64(image: *mut AnyObject) -> Result<String, String> {
        unsafe {
            let tiff: *const AnyObject = msg_send![image, TIFFRepresentation];
            if tiff.is_null() {
                return Err("TIFFRepresentation returned nil".into());
            }
            let rep_alloc: *const AnyObject = msg_send![class!(NSBitmapImageRep), alloc];
            let rep: *const AnyObject = msg_send![rep_alloc, initWithData: tiff];
            if rep.is_null() {
                return Err("NSBitmapImageRep init failed".into());
            }
            // NSBitmapImageFileTypePNG == 4
            let props: *const AnyObject = msg_send![class!(NSDictionary), dictionary];
            let png: *const AnyObject =
                msg_send![rep, representationUsingType: 4usize, properties: &*props];
            if png.is_null() {
                return Err("PNG representation failed".into());
            }
            let len: usize = msg_send![png, length];
            let bytes: *const std::os::raw::c_void = msg_send![png, bytes];
            if len == 0 || bytes.is_null() {
                return Err("empty PNG data".into());
            }
            let data = std::slice::from_raw_parts(bytes as *const u8, len);
            Ok(base64::engine::general_purpose::STANDARD.encode(data))
        }
    }

    /// Drop snapshot-cache entries whose webview no longer exists (issue
    /// #139). `live` is the current WKWebView list from the same hierarchy
    /// walk that found this request's webview.
    fn reap_freed_webview_entries(live: &[*const AnyObject]) {
        let live_keys: std::collections::HashSet<u64> =
            live.iter().map(|p| *p as usize as u64).collect();
        let mut cache = SNAPSHOT_CACHE.lock().unwrap_or_else(|p| p.into_inner());
        cache.retain_live(|key| live_keys.contains(key));
    }

    /// Depth-first search of the NSWindow/view hierarchy for a WKWebView.
    fn find_wkwebview() -> Option<*const AnyObject> {
        live_wkwebviews().into_iter().next()
    }

    /// Every live WKWebView in the NSWindow/view hierarchy (DFS order); the
    /// source of truth for reaping freed webviews' cache entries.
    fn live_wkwebviews() -> Vec<*const AnyObject> {
        let nsapp: *const AnyObject =
            unsafe { msg_send![class!(NSApplication), sharedApplication] };
        if nsapp.is_null() {
            return Vec::new();
        }
        let windows: *const AnyObject = unsafe { msg_send![nsapp, windows] };
        let count: usize = unsafe { msg_send![windows, count] };
        let mut found = Vec::new();
        for i in 0..count {
            let window: *const AnyObject = unsafe { msg_send![windows, objectAtIndex: i] };
            let content: *const AnyObject = unsafe { msg_send![window, contentView] };
            collect_wkwebviews(content, &mut found);
        }
        found
    }

    fn collect_wkwebviews(view: *const AnyObject, out: &mut Vec<*const AnyObject>) {
        if view.is_null() {
            return;
        }
        unsafe {
            // Match WKWebView *or subclasses* (wry uses its own WryWebView subclass).
            let wk_class: *const AnyObject =
                class!(WKWebView) as *const objc2::runtime::AnyClass as *const AnyObject;
            let is_wk: bool = msg_send![view, isKindOfClass: wk_class];
            if is_wk {
                out.push(view);
            }
            let subviews: *const AnyObject = msg_send![view, subviews];
            let count: usize = msg_send![subviews, count];
            for i in 0..count {
                let child: *const AnyObject = msg_send![subviews, objectAtIndex: i];
                collect_wkwebviews(child, out);
            }
        }
    }
}
