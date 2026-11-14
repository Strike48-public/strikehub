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

#[cfg(target_os = "macos")]
mod server {
    use std::io::{BufRead, Write};
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::PathBuf;
    use std::sync::mpsc;
    use std::time::Duration;

    use base64::Engine;
    use block2::{Block, RcBlock};
    use dispatch::Queue;
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send};

    type Resp = Result<serde_json::Value, String>;
    type ObjCFn = dyn Fn(*mut AnyObject, *mut AnyObject);
    type ObjCBlock2 = Block<ObjCFn>;

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
            .name("strikehub-inspect".into())
            .spawn(move || {
                for stream in listener.incoming().flatten() {
                    handle_conn(stream);
                }
            })?;

        Ok(socket_path)
    }

    fn handle_conn(stream: UnixStream) {
        let _ = stream.set_read_timeout(Some(Duration::from_secs(300)));
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
            let Some(wv_ptr) = find_wkwebview() else {
                let _ = tx.send(Err("no WKWebView found (window not up yet?)".into()));
                return;
            };
            let wv: &AnyObject = unsafe { &*wv_ptr };
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
                            let _ = tx.send(Ok(serde_json::json!({ "ok": true, "png_b64": b64 })));
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
        match rx.recv_timeout(Duration::from_secs(60)) {
            Ok(Ok(v)) => v,
            Ok(Err(e)) => serde_json::json!({ "ok": false, "error": e }),
            Err(_) => serde_json::json!({
                "ok": false,
                "error": "internal: main-thread hop timed out (60s)"
            }),
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

    /// Depth-first search of the NSWindow/view hierarchy for a WKWebView.
    fn find_wkwebview() -> Option<*const AnyObject> {
        let nsapp: *const AnyObject =
            unsafe { msg_send![class!(NSApplication), sharedApplication] };
        if nsapp.is_null() {
            return None;
        }
        let windows: *const AnyObject = unsafe { msg_send![nsapp, windows] };
        let count: usize = unsafe { msg_send![windows, count] };
        for i in 0..count {
            let window: *const AnyObject = unsafe { msg_send![windows, objectAtIndex: i] };
            let content: *const AnyObject = unsafe { msg_send![window, contentView] };
            if let Some(w) = find_wkwebview_in_view(content) {
                return Some(w);
            }
        }
        None
    }

    fn find_wkwebview_in_view(view: *const AnyObject) -> Option<*const AnyObject> {
        if view.is_null() {
            return None;
        }
        unsafe {
            // Match WKWebView *or subclasses* (wry uses its own WryWebView subclass).
            let wk_class: *const AnyObject =
                class!(WKWebView) as *const objc2::runtime::AnyClass as *const AnyObject;
            let is_wk: bool = msg_send![view, isKindOfClass: wk_class];
            if is_wk {
                return Some(view);
            }
            let subviews: *const AnyObject = msg_send![view, subviews];
            let count: usize = msg_send![subviews, count];
            for i in 0..count {
                let child: *const AnyObject = msg_send![subviews, objectAtIndex: i];
                if let Some(w) = find_wkwebview_in_view(child) {
                    return Some(w);
                }
            }
        }
        None
    }
}
