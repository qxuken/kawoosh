//! Native extensions: the C ABI the kui way (docs/design/native.md).
//!
//! A shared library `init.lua` names (`kawoosh.extension(namespace[,
//! where])`, found by [`locate`]) is opened here, its `kw_ext_abi` checked against
//! [`KW_ABI_VERSION`], its `kw_ext_init` run with a context. Everything
//! it then asks of the editor goes through one door, `kw_call(ctx, name,
//! args)`: the Lua name of a `kawoosh.*` function and its arguments as a
//! kui value list, answered through the one Lua state — every door
//! exists on day one, with the argument checks and the error messages
//! the Lua one has (native.md Decision 2). Where Lua would take a
//! function the extension passes a handle (`kw_fn`), a Lua function the
//! host makes over a C pointer. A door that shows up hot in a native
//! extension is re-expressed as a `Value`-native function in Rust when
//! a profile says so, and both bindings call it; the protocol does not
//! change.
//!
//! The values are kui-ffi's (`KuiValue`, `KuiStr`): the one value type
//! an extension already builds for `kui_open`, read and built here
//! through the same `kui_value_*` functions a C host uses — the ABI
//! itself, and the only way in from outside kui-ffi, whose `Value` is
//! private to it.
//!
//! The header is `kawoosh/include/kawoosh.h`. Every `kw_*` body is
//! under `catch_unwind` and answers an error rather than unwinding
//! into C; the context handed to C is alive for the one call.

// Safe `extern "C"` functions taking raw pointers is the point of this
// layer, as of kui-ffi's: every entry point null-checks and catches
// panics instead of being `unsafe`.
#![allow(clippy::not_unsafe_ptr_arg_deref)]

use std::borrow::Cow;
use std::cell::RefCell;
use std::ffi::{CStr, c_char, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Mutex, OnceLock};

use kui_ffi::{
    KuiStr, KuiValue, kui_value_as_bool, kui_value_as_float, kui_value_as_int, kui_value_as_str,
    kui_value_at, kui_value_bool, kui_value_entry, kui_value_float, kui_value_free, kui_value_int,
    kui_value_is_null, kui_value_len, kui_value_list, kui_value_list_push, kui_value_map,
    kui_value_map_set, kui_value_null, kui_value_str,
};
use mlua::{Function, Lua, MultiValue, Table, Value as LV};

use crate::{Msg, Published};

/// The ABI this build implements: `KW_ABI_VERSION` in `kawoosh.h`,
/// bumped when a `kw_*` prototype or a `repr(C)` struct moves. An
/// extension built against another is refused at load.
pub const KW_ABI_VERSION: u32 = 3;

/// The doors' version, `kw_protocol`: which `kawoosh.*` names exist and
/// what they take. Moves with kawoosh's releases, never with the ABI.
pub const KW_PROTOCOL: u32 = 1;

/// A callable the extension hands over through `kw_fn`: called on the
/// UI thread with the arguments the door documents, as a list borrowed
/// for the call; what it returns is the host's to free.
pub type KwFn = extern "C" fn(*mut c_void, *mut KwCtx, *const KuiValue) -> *mut KuiValue;
type InitFn = extern "C" fn(*mut KwCtx) -> *mut c_void;
type FreeFn = extern "C" fn(*mut c_void);

/// One edit for `kw_buf_edits`: `from..to` (bytes, `to` exclusive)
/// replaced by `text`. `[in]`: the extension allocates, the host reads;
/// a field appended here is read only by a build that knows it.
#[repr(C)]
pub struct KwEdit {
    pub from: u64,
    pub to: u64,
    pub text: KuiStr,
}

/// A loaded extension: what `kawoosh.extensions()` lists.
pub struct Loaded {
    pub namespace: String,
    pub name: String,
    pub path: PathBuf,
    pub abi: u32,
    /// It exports `kui_ext_abi` too: it draws, through kui's own
    /// loader, as a kui extension under the same namespace (native.md
    /// Decision 4).
    pub kui: bool,
    /// Whatever `kw_ext_init` answered, handed to `kw_ext_free`.
    user: *mut c_void,
    free: Option<FreeFn>,
}

/// The extensions loaded into one runtime, the handles they made and
/// the last error a `kw_*` call left (`kw_error`).
#[derive(Default)]
pub struct Native {
    pub loaded: Vec<Loaded>,
    /// Libraries that export `kui_ext_abi`, not yet added to the frame:
    /// the shell takes them (`take_pending_kui`) on its next frame,
    /// where kui's `Ui::add_extension` can run.
    pending_kui: Vec<(String, PathBuf)>,
    /// A handle with the namespace of the extension that made it.
    fns: Vec<(KwFn, *mut c_void, Option<String>)>,
    /// The Lua function made over each handle, once.
    lua_fns: Vec<Option<Function>>,
    error: String,
    /// The runtime's snapshot and queue, for the typed doors
    /// (`kw_buf_text`, `kw_buf_edits`): what `kawoosh.buf.text` and
    /// `kawoosh.buf.edits` read and push, without a Lua value between.
    published: Option<Rc<RefCell<Published>>>,
    queue: Option<Rc<RefCell<Vec<Msg>>>>,
    /// The thread the runtime was made on: the UI thread, the one a
    /// `kw_*` call may run on.
    thread: Option<std::thread::ThreadId>,
}

pub type NativeCell = Rc<RefCell<Native>>;

impl Drop for Native {
    /// The runtime is going (a config reload makes a new one): every
    /// extension's `kw_ext_free` runs with its state. The library stays
    /// open — a handle's pointer may be reached again — and the next
    /// runtime's `kawoosh.extension` runs `kw_ext_init` anew.
    fn drop(&mut self) {
        for l in self.loaded.drain(..) {
            if let Some(free) = l.free {
                free(l.user);
            }
        }
    }
}

/// The context handed to C for one call: the runtime's Lua, its
/// native state and the namespace of the extension whose call it is
/// (none for a wake's). Opaque to C, alive for the call it was made for.
pub struct KwCtx {
    native: NativeCell,
    lua: Lua,
    namespace: Option<String>,
    /// What `kw_buf_text` answered, kept for the call: the strings it
    /// handed out point into these.
    texts: RefCell<Vec<String>>,
    /// The UI thread, copied out so a call from another thread is
    /// caught before anything behind the `Rc` is touched.
    thread: std::thread::ThreadId,
}

impl KwCtx {
    /// Whether this call is on the UI thread. A call from any other
    /// answers NULL or false and sets no error — nothing on that thread
    /// may touch the runtime, the error slot included.
    fn on_thread(&self) -> bool {
        std::thread::current().id() == self.thread
    }

    /// The start of a `kw_*` call that can fail: the last error
    /// cleared, so `kw_error` after it is this call's.
    fn enter(&self) -> bool {
        if !self.on_thread() {
            return false;
        }
        self.native.borrow_mut().error.clear();
        true
    }
}

fn with_ctx<T>(
    native: &NativeCell,
    lua: &Lua,
    namespace: Option<&str>,
    f: impl FnOnce(*mut KwCtx) -> T,
) -> T {
    let thread = native
        .borrow()
        .thread
        .unwrap_or_else(|| std::thread::current().id());
    let mut ctx = KwCtx {
        native: native.clone(),
        lua: lua.clone(),
        namespace: namespace.map(str::to_string),
        texts: RefCell::new(Vec::new()),
        thread,
    };
    f(&mut ctx)
}

/// A call queued from a thread (`kw_wake`), run on the UI thread.
struct Wake(KwFn, *mut c_void);
// SAFETY: the pointer crosses threads by the header's contract — what
// `kw_wake` is for — and is only ever handed back to the function
// beside it.
unsafe impl Send for Wake {}

static WAKES: Mutex<Vec<Wake>> = Mutex::new(Vec::new());
static WAKER: OnceLock<Mutex<Option<kawoosh_systems::WakeHandle>>> = OnceLock::new();

/// The shell's wake handle, so a `kw_wake` from a thread brings a
/// frame; before it is set, a wake queues and the next frame runs it.
pub fn set_waker(handle: kawoosh_systems::WakeHandle) {
    *WAKER.get_or_init(|| Mutex::new(None)).lock().unwrap() = Some(handle);
}

/// Whether a wake is queued and not yet run.
pub fn wakes_pending() -> bool {
    !WAKES.lock().unwrap().is_empty()
}

/// Runs every queued wake on this thread, each with a context of its
/// own and no namespace; what one returns is freed. How many ran.
pub fn run_wakes(native: &NativeCell, lua: &Lua) -> usize {
    let wakes: Vec<Wake> = std::mem::take(&mut *WAKES.lock().unwrap());
    let n = wakes.len();
    for Wake(f, user) in wakes {
        let out = with_ctx(native, lua, None, |c| f(user, c, std::ptr::null()));
        kui_value_free(out);
    }
    n
}

fn guard<T>(default: T, f: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(default)
}

fn bytes_of<'a>(s: KuiStr) -> &'a [u8] {
    if s.ptr.is_null() || s.len == 0 {
        &[]
    } else {
        // SAFETY: a `KuiStr` is `len` bytes at `ptr`, by the header's
        // contract; a null one is the empty string.
        unsafe { std::slice::from_raw_parts(s.ptr, s.len) }
    }
}

fn str_of<'a>(s: KuiStr) -> Cow<'a, str> {
    String::from_utf8_lossy(bytes_of(s))
}

fn kstr(s: &str) -> KuiStr {
    KuiStr {
        ptr: s.as_ptr(),
        len: s.len(),
    }
}

// ---- the four `kw_*` functions, as `kawoosh.h` declares them

/// `kw_call(ctx, name, args)`: `kawoosh.NAME(args...)` with a C calling
/// convention. Answers the return value (a list of them when the door
/// returns several), the caller's to free, or NULL with the reason in
/// `kw_error` — a door that answers `nil, why` has refused, and that
/// is NULL and `why` too.
#[unsafe(no_mangle)]
pub extern "C" fn kw_call(ctx: *mut KwCtx, name: KuiStr, args: *const KuiValue) -> *mut KuiValue {
    guard(std::ptr::null_mut(), || {
        // SAFETY: a context is one `with_ctx` made for this call, or
        // null, which the header says answers NULL.
        let Some(ctx) = (unsafe { ctx.as_ref() }) else {
            return std::ptr::null_mut();
        };
        if !ctx.enter() {
            return std::ptr::null_mut();
        }
        let name = str_of(name).into_owned();
        match call(ctx, &name, args) {
            Ok(v) => v,
            Err(e) => {
                ctx.native.borrow_mut().error = e;
                std::ptr::null_mut()
            }
        }
    })
}

/// `kw_error(ctx, out)`: why the last `kw_call`, `kw_fn` or `kw_buf_*`
/// on this context answered NULL or false, borrowed until the next of
/// them; false with nothing written when it succeeded.
#[unsafe(no_mangle)]
pub extern "C" fn kw_error(ctx: *mut KwCtx, out: *mut KuiStr) -> bool {
    guard(false, || {
        // SAFETY: as `kw_call`'s; `out` is the caller's to write or null.
        let (Some(ctx), Some(out)) = (unsafe { ctx.as_ref() }, unsafe { out.as_mut() }) else {
            return false;
        };
        if !ctx.on_thread() {
            return false;
        }
        let n = ctx.native.borrow();
        if n.error.is_empty() {
            return false;
        }
        *out = kstr(&n.error);
        true
    })
}

/// `kw_protocol(ctx)`: the doors' version ([`KW_PROTOCOL`]); 0 on no
/// context.
#[unsafe(no_mangle)]
pub extern "C" fn kw_protocol(ctx: *mut KwCtx) -> u32 {
    if ctx.is_null() { 0 } else { KW_PROTOCOL }
}

/// `kw_namespace(ctx, out)`: the namespace the extension whose call
/// this is was loaded under, borrowed for the call; false with nothing
/// written on a wake's context or a null one.
#[unsafe(no_mangle)]
pub extern "C" fn kw_namespace(ctx: *mut KwCtx, out: *mut KuiStr) -> bool {
    guard(false, || {
        // SAFETY: as `kw_call`'s; `out` is the caller's to write or null.
        let (Some(ctx), Some(out)) = (unsafe { ctx.as_ref() }, unsafe { out.as_mut() }) else {
            return false;
        };
        match &ctx.namespace {
            Some(ns) => {
                *out = kstr(ns);
                true
            }
            None => false,
        }
    })
}

/// `kw_buf_text(ctx, buffer, out)`: the buffer's text (0 = the current
/// one), one copy — the snapshot's — borrowed until the call returns;
/// false with the reason in `kw_error` for a buffer that is not.
#[unsafe(no_mangle)]
pub extern "C" fn kw_buf_text(ctx: *mut KwCtx, buffer: u64, out: *mut KuiStr) -> bool {
    guard(false, || {
        // SAFETY: as `kw_call`'s; `out` is the caller's to write or null.
        let (Some(ctx), Some(out)) = (unsafe { ctx.as_ref() }, unsafe { out.as_mut() }) else {
            return false;
        };
        if !ctx.enter() {
            return false;
        }
        match snapshot_text(ctx, buffer) {
            Ok(text) => {
                let mut texts = ctx.texts.borrow_mut();
                texts.push(text);
                *out = kstr(texts.last().unwrap());
                true
            }
            Err(e) => {
                ctx.native.borrow_mut().error = e;
                false
            }
        }
    })
}

/// `kw_buf_edits(ctx, buffer, edits, n)`: `n` edits applied to the
/// buffer (0 = the current one) as `kawoosh.buf.edits` applies a list
/// of them — at once, each range in the text before, none overlapping —
/// with no Lua table between. False with the reason in `kw_error`.
#[unsafe(no_mangle)]
pub extern "C" fn kw_buf_edits(
    ctx: *mut KwCtx,
    buffer: u64,
    edits: *const KwEdit,
    n: usize,
) -> bool {
    guard(false, || {
        // SAFETY: as `kw_call`'s; `edits` is `n` structs the caller
        // laid out, by the header's contract, or null for none.
        let Some(ctx) = (unsafe { ctx.as_ref() }) else {
            return false;
        };
        if !ctx.enter() {
            return false;
        }
        let edits = if edits.is_null() || n == 0 {
            &[][..]
        } else {
            unsafe { std::slice::from_raw_parts(edits, n) }
        };
        match push_edits(ctx, buffer, edits) {
            Ok(()) => true,
            Err(e) => {
                ctx.native.borrow_mut().error = e;
                false
            }
        }
    })
}

fn buffer_of(ctx: &KwCtx, buffer: u64) -> Result<u64, String> {
    let n = ctx.native.borrow();
    let p = n
        .published
        .as_ref()
        .ok_or_else(|| "no runtime".to_string())?
        .borrow();
    let h = if buffer == 0 { p.current } else { Some(buffer) };
    let h = h.ok_or_else(|| "no current buffer".to_string())?;
    if !p.buffers.contains_key(&h) {
        return Err(format!("no buffer {h}"));
    }
    Ok(h)
}

fn snapshot_text(ctx: &KwCtx, buffer: u64) -> Result<String, String> {
    let h = buffer_of(ctx, buffer)?;
    let n = ctx.native.borrow();
    let p = n.published.as_ref().unwrap().borrow();
    Ok(p.buffers[&h].snapshot.text())
}

fn push_edits(ctx: &KwCtx, buffer: u64, edits: &[KwEdit]) -> Result<(), String> {
    let h = buffer_of(ctx, buffer)?;
    let mut list = Vec::with_capacity(edits.len());
    for e in edits {
        let (from, to) = (e.from as usize, e.to as usize);
        if to < from {
            return Err(format!("edits: {from}..{to} ends before it starts"));
        }
        list.push((from..to, str_of(e.text).into_owned()));
    }
    crate::check_edits(&list)?;
    let n = ctx.native.borrow();
    n.queue
        .as_ref()
        .ok_or_else(|| "no runtime".to_string())?
        .borrow_mut()
        .push(Msg::Edits {
            buffer: h,
            edits: list,
            carets: Vec::new(),
            primary: 0,
        });
    Ok(())
}

/// `kw_wake(fn, user)`: `fn(user, ctx, NULL)` on the UI thread, soon —
/// the frame is woken. Callable from any thread, the one `kw_*` that
/// is; a null `fn` does nothing.
#[unsafe(no_mangle)]
pub extern "C" fn kw_wake(f: Option<KwFn>, user: *mut c_void) {
    guard((), || {
        let Some(f) = f else { return };
        WAKES.lock().unwrap().push(Wake(f, user));
        if let Some(w) = WAKER.get().and_then(|w| w.lock().unwrap().clone()) {
            w.wake();
        }
    })
}

/// `kw_fn(ctx, fn, user)`: a callable value — a map with the one key
/// `kw_fn` — to put where Lua would put a function. Lives until the
/// runtime goes. NULL for a null `fn`.
#[unsafe(no_mangle)]
pub extern "C" fn kw_fn(ctx: *mut KwCtx, f: Option<KwFn>, user: *mut c_void) -> *mut KuiValue {
    guard(std::ptr::null_mut(), || {
        // SAFETY: as `kw_call`'s.
        let Some(ctx) = (unsafe { ctx.as_ref() }) else {
            return std::ptr::null_mut();
        };
        if !ctx.enter() {
            return std::ptr::null_mut();
        }
        let Some(f) = f else {
            ctx.native.borrow_mut().error = "kw_fn: a null function".into();
            return std::ptr::null_mut();
        };
        // The same function with the same `user` from the same
        // extension is the same handle: a handle made per event or per
        // spawn costs nothing after the first.
        let id = {
            let mut n = ctx.native.borrow_mut();
            let same = n.fns.iter().position(|(g, u, ns)| {
                *g as usize == f as usize && *u == user && *ns == ctx.namespace
            });
            match same {
                Some(id) => id,
                None => {
                    n.fns.push((f, user, ctx.namespace.clone()));
                    n.lua_fns.push(None);
                    n.fns.len() - 1
                }
            }
        };
        let m = kui_value_map();
        kui_value_map_set(m, kstr(HANDLE_KEY), kui_value_int(id as i64));
        m
    })
}

/// The one key of a handle's map.
const HANDLE_KEY: &str = "kw_fn";

fn call(ctx: &KwCtx, name: &str, args: *const KuiValue) -> Result<*mut KuiValue, String> {
    let f = door(&ctx.lua, name)?;
    let mut lua_args = MultiValue::new();
    if !args.is_null() {
        if is_scalar(args) {
            return Err(format!("`{name}`: the arguments are not a list"));
        }
        let n = kui_value_len(args);
        if n > 0 && kui_value_at(args, 0).is_null() {
            return Err(format!("`{name}`: the arguments are a map, not a list"));
        }
        for i in 0..n {
            lua_args.push_back(
                to_lua(ctx, kui_value_at(args, i)).map_err(|e| format!("`{name}`: {e}"))?,
            );
        }
    }
    let out: MultiValue = f.call(lua_args).map_err(|e| format!("`{name}`: {e}"))?;
    // A refusal, the way every refusing door speaks: `nil, why`.
    if out.len() == 2
        && let (LV::Nil, LV::String(why)) = (&out[0], &out[1])
    {
        return Err(format!("`{name}`: {}", why.to_string_lossy()));
    }
    Ok(match out.len() {
        0 => kui_value_null(),
        1 => to_c(&out[0]),
        _ => {
            let list = kui_value_list();
            for v in out.iter() {
                kui_value_list_push(list, to_c(v));
            }
            list
        }
    })
}

/// `kawoosh.NAME` by its dotted name, which must end at a function.
fn door(lua: &Lua, name: &str) -> Result<Function, String> {
    let missing = || format!("no door `{name}`");
    if name.is_empty() {
        return Err(missing());
    }
    let mut t: Table = lua.globals().get("kawoosh").map_err(|e| e.to_string())?;
    let mut parts = name.split('.').peekable();
    while let Some(part) = parts.next() {
        let v: LV = t.get(part).map_err(|e| e.to_string())?;
        match (v, parts.peek().is_some()) {
            (LV::Function(f), false) => return Ok(f),
            (LV::Table(inner), true) => t = inner,
            _ => return Err(missing()),
        }
    }
    Err(missing())
}

fn is_scalar(v: *const KuiValue) -> bool {
    let (mut b, mut i, mut f) = (false, 0i64, 0f64);
    let mut s = KuiStr {
        ptr: std::ptr::null(),
        len: 0,
    };
    kui_value_is_null(v)
        || kui_value_as_bool(v, &mut b)
        || kui_value_as_int(v, &mut i)
        || kui_value_as_float(v, &mut f)
        || kui_value_as_str(v, &mut s)
}

/// A kui value as Lua data: a list is a sequence, a map a table with
/// string keys, a handle (`kw_fn`) the Lua function over its pointer.
fn to_lua(ctx: &KwCtx, v: *const KuiValue) -> mlua::Result<LV> {
    if v.is_null() || kui_value_is_null(v) {
        return Ok(LV::Nil);
    }
    let mut b = false;
    if kui_value_as_bool(v, &mut b) {
        return Ok(LV::Boolean(b));
    }
    let mut i = 0i64;
    if kui_value_as_int(v, &mut i) {
        return Ok(LV::Integer(i));
    }
    let mut f = 0f64;
    if kui_value_as_float(v, &mut f) {
        return Ok(LV::Number(f));
    }
    let mut s = KuiStr {
        ptr: std::ptr::null(),
        len: 0,
    };
    if kui_value_as_str(v, &mut s) {
        return Ok(LV::String(ctx.lua.create_string(bytes_of(s))?));
    }
    let n = kui_value_len(v);
    let mut key = KuiStr {
        ptr: std::ptr::null(),
        len: 0,
    };
    let first = kui_value_entry(v, 0, &mut key);
    if !first.is_null() {
        if n == 1 && str_of(key) == HANDLE_KEY {
            let mut id = 0i64;
            if kui_value_as_int(first, &mut id) {
                return handle(ctx, id);
            }
        }
        let t = ctx.lua.create_table()?;
        for j in 0..n {
            let e = kui_value_entry(v, j, &mut key);
            t.set(ctx.lua.create_string(bytes_of(key))?, to_lua(ctx, e)?)?;
        }
        return Ok(LV::Table(t));
    }
    let t = ctx.lua.create_table()?;
    for j in 0..n {
        t.set(j + 1, to_lua(ctx, kui_value_at(v, j))?)?;
    }
    Ok(LV::Table(t))
}

/// The Lua function over handle `id`, made once: it converts its
/// arguments to a list, calls the C function with a context for the
/// call, frees the list and what came back once converted.
fn handle(ctx: &KwCtx, id: i64) -> mlua::Result<LV> {
    let id = usize::try_from(id).map_err(|_| mlua::Error::runtime("kw_fn: no such handle"))?;
    if let Some(Some(f)) = ctx.native.borrow().lua_fns.get(id) {
        return Ok(LV::Function(f.clone()));
    }
    let (f, user, namespace) = ctx
        .native
        .borrow()
        .fns
        .get(id)
        .cloned()
        .ok_or_else(|| mlua::Error::runtime("kw_fn: no such handle"))?;
    let native = ctx.native.clone();
    let lf = ctx.lua.create_function(move |lua, args: MultiValue| {
        let list = kui_value_list();
        for a in args.iter() {
            kui_value_list_push(list, to_c(a));
        }
        let (out, v) = with_ctx(&native, lua, namespace.as_deref(), |c| {
            let out = f(user, c, list);
            // SAFETY: `c` is the context made for this call, alive here.
            let v = to_lua(unsafe { &*c }, out);
            (out, v)
        });
        kui_value_free(list);
        kui_value_free(out);
        v
    })?;
    ctx.native.borrow_mut().lua_fns[id] = Some(lf.clone());
    Ok(LV::Function(lf))
}

/// Lua data as a new kui value, the caller's: a sequence is a list, any
/// other table a map with its keys as strings; a function, userdata or
/// thread is null.
fn to_c(v: &LV) -> *mut KuiValue {
    match v {
        LV::Nil => kui_value_null(),
        LV::Boolean(b) => kui_value_bool(*b),
        LV::Integer(i) => kui_value_int(*i),
        LV::Number(n) => kui_value_float(*n),
        LV::String(s) => {
            let b = s.as_bytes();
            kui_value_str(KuiStr {
                ptr: b.as_ptr(),
                len: b.len(),
            })
        }
        LV::Table(t) => {
            let n = t.raw_len();
            let mut count = 0usize;
            let mut is_list = true;
            for pair in t.clone().pairs::<LV, LV>() {
                let Ok((k, _)) = pair else { continue };
                count += 1;
                match k {
                    LV::Integer(i) if i >= 1 && (i as usize) <= n => {}
                    _ => {
                        is_list = false;
                        break;
                    }
                }
            }
            if is_list && count == n {
                let list = kui_value_list();
                for i in 1..=n {
                    let item: LV = t.raw_get(i).unwrap_or(LV::Nil);
                    kui_value_list_push(list, to_c(&item));
                }
                list
            } else {
                let map = kui_value_map();
                for pair in t.clone().pairs::<LV, LV>() {
                    let Ok((k, val)) = pair else { continue };
                    let key = match &k {
                        LV::String(s) => String::from_utf8_lossy(&s.as_bytes()).into_owned(),
                        LV::Integer(i) => i.to_string(),
                        LV::Number(f) => f.to_string(),
                        LV::Boolean(b) => b.to_string(),
                        _ => continue,
                    };
                    kui_value_map_set(map, kstr(&key), to_c(&val));
                }
                map
            }
        }
        _ => kui_value_null(),
    }
}

// ---- loading

/// A namespace is a word with no `/`: kui's rule for a slot's prefix.
pub fn check_namespace(namespace: &str) -> Result<(), String> {
    if namespace.is_empty() || namespace.contains('/') {
        return Err(format!("`{namespace}`: a namespace is a word with no `/`"));
    }
    Ok(())
}

/// Where the library for `namespace` is (native.md Decision 7): with
/// nothing said, `ext/NAMESPACE.<ext>` under the config directory,
/// `.so` accepted on any platform as grammars are named; a directory
/// said holds `NAMESPACE.<ext>`; a path said without its extension
/// gets the platform's; a file said is the file. Not there: the places
/// looked, in the error.
pub fn locate(
    namespace: &str,
    said: Option<&Path>,
    config: Option<&Path>,
) -> Result<PathBuf, String> {
    let ext = std::env::consts::DLL_EXTENSION;
    let mut names = vec![format!("{namespace}.{ext}")];
    if ext != "so" {
        names.push(format!("{namespace}.so"));
    }
    let cands: Vec<PathBuf> = match said {
        Some(p) if p.is_file() => return Ok(p.to_path_buf()),
        Some(p) if p.is_dir() => names.iter().map(|n| p.join(n)).collect(),
        Some(p) => {
            let mut c = vec![p.with_extension(ext)];
            if ext != "so" {
                c.push(p.with_extension("so"));
            }
            c.push(p.to_path_buf());
            c
        }
        None => {
            let Some(config) = config else {
                return Err(format!("`{namespace}`: no config directory to look under"));
            };
            names.iter().map(|n| config.join("ext").join(n)).collect()
        }
    };
    cands.iter().find(|p| p.is_file()).cloned().ok_or_else(|| {
        let looked: Vec<String> = cands.iter().map(|p| p.display().to_string()).collect();
        format!("`{namespace}`: no extension at {}", looked.join(", "))
    })
}

/// Opens the library at `path` as `namespace` and runs its
/// `kw_ext_init`. Refused, with the reason: a namespace that is empty,
/// has a `/` or is another library's; a library that will not load;
/// one that declares no `kw_ext_abi` or another [`KW_ABI_VERSION`]. The
/// same path under the same namespace again is already loaded, and
/// answers Ok. The library stays open for the process (native.md
/// Decision 7).
pub fn load(native: &NativeCell, lua: &Lua, namespace: &str, path: &Path) -> Result<(), String> {
    check_namespace(namespace)?;
    if let Some(prev) = native
        .borrow()
        .loaded
        .iter()
        .find(|l| l.namespace == namespace)
    {
        if prev.path == path {
            return Ok(());
        }
        return Err(format!(
            "`{namespace}` is already {}; a second extension needs a second namespace",
            prev.path.display()
        ));
    }
    let shown = path.display();
    // SAFETY: the library's code runs in this process on this frame;
    // `init.lua` naming one is trusting it as it trusts the Lua that
    // names it, which is `kawoosh.spawn` away from anything.
    let lib = unsafe { libloading::Library::new(path) }.map_err(|e| format!("{shown}: {e}"))?;
    // SAFETY (each `get`): the type is the signature `kawoosh.h`
    // declares the symbol with.
    let abi = unsafe { lib.get::<extern "C" fn() -> u32>(b"kw_ext_abi\0") }
        .map(|s| *s)
        .map_err(|_| {
            format!("{shown}: extension declares no ABI; this build is {KW_ABI_VERSION}")
        })?;
    let claimed = abi();
    if claimed != KW_ABI_VERSION {
        return Err(format!(
            "{shown}: extension is ABI {claimed}, this build is {KW_ABI_VERSION}"
        ));
    }
    let name = unsafe { lib.get::<extern "C" fn() -> *const c_char>(b"kw_ext_name\0") }
        .ok()
        .map(|s| (*s)())
        .filter(|p| !p.is_null())
        // SAFETY: `kw_ext_name` answers a NUL-terminated static string.
        .map(|p| unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| {
            path.file_stem()
                .map_or_else(|| shown.to_string(), |s| s.to_string_lossy().into_owned())
        });
    let init = unsafe { lib.get::<InitFn>(b"kw_ext_init\0") }
        .ok()
        .map(|s| *s);
    let free = unsafe { lib.get::<FreeFn>(b"kw_ext_free\0") }
        .ok()
        .map(|s| *s);
    let kui = unsafe { lib.get::<extern "C" fn() -> u32>(b"kui_ext_abi\0") }.is_ok();
    // Never unloaded: a handle's pointer, a Lua function over it, a
    // string the extension answered may all be reached again.
    std::mem::forget(lib);
    // Last, so an init that allocates does so once every check has
    // passed and `free` is already known to undo it.
    let user = init.map_or(std::ptr::null_mut(), |init| {
        with_ctx(native, lua, Some(namespace), |c| init(c))
    });
    let mut n = native.borrow_mut();
    if kui {
        n.pending_kui
            .push((namespace.to_string(), path.to_path_buf()));
    }
    n.loaded.push(Loaded {
        namespace: namespace.to_string(),
        name,
        path: path.to_path_buf(),
        abi: claimed,
        kui,
        user,
        free,
    });
    Ok(())
}

/// The libraries loaded since the last call that draw as kui
/// extensions, for the shell to add to the frame under their
/// namespaces.
pub fn take_pending_kui(native: &NativeCell) -> Vec<(String, PathBuf)> {
    std::mem::take(&mut native.borrow_mut().pending_kui)
}

/// The two doors, on the `kawoosh` table: the boot script's
/// `kawoosh.extension` wraps the first. The snapshot and the queue are
/// the typed doors'.
pub fn seed(
    lua: &Lua,
    native: &NativeCell,
    published: &Rc<RefCell<Published>>,
    queue: &Rc<RefCell<Vec<Msg>>>,
) -> mlua::Result<()> {
    {
        let mut n = native.borrow_mut();
        n.published = Some(published.clone());
        n.queue = Some(queue.clone());
        n.thread = Some(std::thread::current().id());
    }
    let k: Table = lua.globals().get("kawoosh")?;
    let n = native.clone();
    // `kawoosh._extension(namespace[, where])`: the library `locate`
    // finds loaded as `namespace` (`kawoosh.extension` is the one to
    // call — it expands `where` and says why a load was refused);
    // `true`, or `nil` and the reason.
    k.set(
        "_extension",
        lua.create_function(move |lua, (namespace, said): (String, Option<String>)| {
            let config = kawoosh_systems::fs::config_dir();
            let found = check_namespace(&namespace).and_then(|()| {
                locate(
                    &namespace,
                    said.as_deref().map(Path::new),
                    config.as_deref(),
                )
            });
            match found.and_then(|path| load(&n, lua, &namespace, &path)) {
                Ok(()) => Ok((Some(true), None)),
                Err(e) => Ok((None, Some(e))),
            }
        })?,
    )?;
    let n = native.clone();
    // `kawoosh.extensions()`: the native extensions loaded, in the
    // order they were, each `{ namespace, name, path, abi, protocol }`.
    k.set(
        "extensions",
        lua.create_function(move |lua, ()| {
            let list = lua.create_table()?;
            for (i, l) in n.borrow().loaded.iter().enumerate() {
                let t = lua.create_table()?;
                t.set("namespace", l.namespace.as_str())?;
                t.set("name", l.name.as_str())?;
                t.set("path", l.path.to_string_lossy().into_owned())?;
                t.set("abi", l.abi)?;
                t.set("protocol", KW_PROTOCOL)?;
                t.set("draws", l.kui)?;
                list.set(i + 1, t)?;
            }
            Ok(list)
        })?,
    )?;
    Ok(())
}
