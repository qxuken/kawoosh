//! Native extensions: the C ABI the kui way (docs/design/native.md).
//!
//! A shared library `init.lua` names (`kawoosh.extension(namespace,
//! path)`) is opened here, its `kw_ext_abi` checked against
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

use kui_ffi::{
    KuiStr, KuiValue, kui_value_as_bool, kui_value_as_float, kui_value_as_int, kui_value_as_str,
    kui_value_at, kui_value_bool, kui_value_entry, kui_value_float, kui_value_free, kui_value_int,
    kui_value_is_null, kui_value_len, kui_value_list, kui_value_list_push, kui_value_map,
    kui_value_map_set, kui_value_null, kui_value_str,
};
use mlua::{Function, Lua, MultiValue, Table, Value as LV};

/// The ABI this build implements: `KW_ABI_VERSION` in `kawoosh.h`,
/// bumped when a `kw_*` prototype or a `repr(C)` struct moves. An
/// extension built against another is refused at load.
pub const KW_ABI_VERSION: u32 = 1;

/// The doors' version, `kw_protocol`: which `kawoosh.*` names exist and
/// what they take. Moves with kawoosh's releases, never with the ABI.
pub const KW_PROTOCOL: u32 = 1;

/// A callable the extension hands over through `kw_fn`: called on the
/// UI thread with the arguments the door documents, as a list borrowed
/// for the call; what it returns is the host's to free.
pub type KwFn = extern "C" fn(*mut c_void, *mut KwCtx, *const KuiValue) -> *mut KuiValue;
type InitFn = extern "C" fn(*mut KwCtx) -> *mut c_void;
type FreeFn = extern "C" fn(*mut c_void);

/// A loaded extension: what `kawoosh.extensions()` lists.
pub struct Loaded {
    pub namespace: String,
    pub name: String,
    pub path: PathBuf,
    pub abi: u32,
    /// Whatever `kw_ext_init` answered, handed to `kw_ext_free`.
    user: *mut c_void,
    free: Option<FreeFn>,
}

/// The extensions loaded into one runtime, the handles they made and
/// the last error a `kw_*` call left (`kw_error`).
#[derive(Default)]
pub struct Native {
    pub loaded: Vec<Loaded>,
    fns: Vec<(KwFn, *mut c_void)>,
    /// The Lua function made over each handle, once.
    lua_fns: Vec<Option<Function>>,
    error: String,
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

/// The context handed to C for one call: the runtime's Lua and its
/// native state. Opaque to C, alive for the call it was made for.
pub struct KwCtx {
    native: NativeCell,
    lua: Lua,
}

fn with_ctx<T>(native: &NativeCell, lua: &Lua, f: impl FnOnce(*mut KwCtx) -> T) -> T {
    let mut ctx = KwCtx {
        native: native.clone(),
        lua: lua.clone(),
    };
    f(&mut ctx)
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
/// `kw_error`.
#[unsafe(no_mangle)]
pub extern "C" fn kw_call(ctx: *mut KwCtx, name: KuiStr, args: *const KuiValue) -> *mut KuiValue {
    guard(std::ptr::null_mut(), || {
        // SAFETY: a context is one `with_ctx` made for this call, or
        // null, which the header says answers NULL.
        let Some(ctx) = (unsafe { ctx.as_ref() }) else {
            return std::ptr::null_mut();
        };
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

/// `kw_error(ctx, out)`: the last error a `kw_*` call on this context
/// left, borrowed until the next; false with nothing written when
/// there is none.
#[unsafe(no_mangle)]
pub extern "C" fn kw_error(ctx: *mut KwCtx, out: *mut KuiStr) -> bool {
    guard(false, || {
        // SAFETY: as `kw_call`'s; `out` is the caller's to write or null.
        let (Some(ctx), Some(out)) = (unsafe { ctx.as_ref() }, unsafe { out.as_mut() }) else {
            return false;
        };
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
        let Some(f) = f else {
            ctx.native.borrow_mut().error = "kw_fn: a null function".into();
            return std::ptr::null_mut();
        };
        let id = {
            let mut n = ctx.native.borrow_mut();
            n.fns.push((f, user));
            n.lua_fns.push(None);
            n.fns.len() - 1
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
    let (f, user) = ctx
        .native
        .borrow()
        .fns
        .get(id)
        .copied()
        .ok_or_else(|| mlua::Error::runtime("kw_fn: no such handle"))?;
    let native = ctx.native.clone();
    let lf = ctx.lua.create_function(move |lua, args: MultiValue| {
        let list = kui_value_list();
        for a in args.iter() {
            kui_value_list_push(list, to_c(a));
        }
        let (out, v) = with_ctx(&native, lua, |c| {
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

/// Opens the library at `path` as `namespace` and runs its
/// `kw_ext_init`. Refused, with the reason: a namespace that is empty,
/// has a `/` or is another library's; a library that will not load;
/// one that declares no `kw_ext_abi` or another [`KW_ABI_VERSION`]. The
/// same path under the same namespace again is already loaded, and
/// answers Ok. The library stays open for the process (native.md
/// Decision 7).
pub fn load(native: &NativeCell, lua: &Lua, namespace: &str, path: &Path) -> Result<(), String> {
    if namespace.is_empty() || namespace.contains('/') {
        return Err(format!("`{namespace}`: a namespace is a word with no `/`"));
    }
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
    // Never unloaded: a handle's pointer, a Lua function over it, a
    // string the extension answered may all be reached again.
    std::mem::forget(lib);
    // Last, so an init that allocates does so once every check has
    // passed and `free` is already known to undo it.
    let user = init.map_or(std::ptr::null_mut(), |init| {
        with_ctx(native, lua, |c| init(c))
    });
    native.borrow_mut().loaded.push(Loaded {
        namespace: namespace.to_string(),
        name,
        path: path.to_path_buf(),
        abi: claimed,
        user,
        free,
    });
    Ok(())
}

/// The two doors, on the `kawoosh` table: the boot script's
/// `kawoosh.extension` wraps the first.
pub fn seed(lua: &Lua, native: &NativeCell) -> mlua::Result<()> {
    let k: Table = lua.globals().get("kawoosh")?;
    let n = native.clone();
    // `kawoosh._extension(namespace, path)`: the library at `path`
    // loaded as `namespace` (`kawoosh.extension` is the one to call —
    // it expands the path and says why a load was refused); `true`, or
    // `nil` and the reason.
    k.set(
        "_extension",
        lua.create_function(move |lua, (namespace, path): (String, String)| {
            match load(&n, lua, &namespace, Path::new(&path)) {
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
                list.set(i + 1, t)?;
            }
            Ok(list)
        })?,
    )?;
    Ok(())
}
