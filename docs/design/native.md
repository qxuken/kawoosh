# Native extensions: a C ABI, the kui way

Status: asked 2026-10-08, "how expensive would be to open kawoosh for
native extension?", three ways costed (Lua C modules, `abi_stable`, an
ABI of kawoosh's own) and the third chosen: "option 3 is the way, let's
design, estimate this". The estimate is at the end. **Round one built
2026-10-08**, "let's get building"; the block after the estimate says
what the building changed. The calls below are taken here, each the
user's to overturn. Companion
to [plugin-panes.md](plugin-panes.md) (what a Lua plugin can do — a
native one can do the same, by the same names), [lua-boundary.md](lua-boundary.md)
(mechanism is Rust's, a plugin's shape is Lua's) and kui's ADR 0006
(the C ABI has a version and the structs it writes carry their size)
and ADR 0014 (slots: an extension fills a place the host declares).
parked.md listed native extensions as out of scope "because there is no
stable Rust ABI"; this note is why that reason does not hold.

## What there is

**Every `kawoosh.*` call is already a message or a snapshot read.** A
plugin's write pushes a `Msg` (102 variants, `lua/src/lib.rs`) that the
app drains between frames; its read looks into `published`, the frame's
snapshot of buffers, panes and settings. The 262 names the bundled
plugins use are mlua closures that convert Lua values to one or the
other. Nothing in that protocol is Lua's: a second front end converts
from something else to the same `Msg` and the same snapshot.

**Every pane that is not an editor or a terminal is a kui slot.** A Lua
view is `Content::Lua(name)`, drawn as `ui.slot_with("lua/NAME@PANE",
params)` (`scripting.rs`, `headers.rs`), and the one `LuaExtension`
fills every slot under `lua/` (kui's `"*"`). Its events come back with
`ev.slot`, and `boot.lua` routes them to the view's handler.

**kui already loads C extensions.** `kui_ffi::CExtension::open` (504
lines) dlopens a library, refuses one with no `kui_ext_abi`, another
ABI or no `kui_ext_view`, reads its slots and name, runs its init;
`Ui::add_extension(namespace, ext)` puts it in the frame's list, a
namespace of its own; `kui.h` (4k lines, ABI 26) is the header, with
`KuiStr`, `KuiValue` (map, list, str, int, float, bool, null; 19
functions) and the seven `kui_ext_*` entry points; `abi_parity`
(`kui-ffi/build.rs`) regenerates `_Static_assert`s from the Rust layout
so the hand-written header cannot drift. `kui-ffi` is in kawoosh's graph
through `kui-lua` (alpha.41). The cost of all of this to kawoosh is a
`#include`.

**The one Lua state is safe.** `Lua::new()` disables `package.loadlib`
and the C searcher (mlua's safe mode). This design does not change
that: a native extension never sees a `lua_State`.

## Decisions

### 1. The ABI is kui's, with a kawoosh family beside it

One header, `kawoosh/include/kawoosh.h`, that includes `kui.h`. It adds
no second string or value type: `KuiStr` and `KuiValue` are the
strings and the data, so a value an extension builds for a kawoosh
door is the value it would build for `kui_open`. It adds a second
family of entry points the extension defines —

```c
KW_EXT_EXPORT uint32_t kw_ext_abi(void);        /* REQUIRED: KW_ABI_VERSION */
KW_EXT_EXPORT const char *kw_ext_name(void);    /* for logs; absent = file stem */
KW_EXT_EXPORT void *kw_ext_init(KwCtx *ctx);    /* register commands, maps, views, hooks */
KW_EXT_EXPORT void kw_ext_free(void *user);     /* at exit; libraries never unload */
```

— and a second version, `KW_ABI_VERSION`, checked at load as
`KUI_ABI_VERSION` is. A library that also exports the `kui_ext_*` six
draws (Decision 4); one that exports only `kw_ext_*` is a command-line
extension and draws nothing. `KW_EXT_EXPORT` is `KUI_EXT_EXPORT`.

The four directions of ADR 0006 are audited per struct in the header
(**[in]** host reads, append-only; **[out]** host allocates, the
library writes, size-prefixed; **[out-array]**; **[lib]**), and
`abi_parity` is copied from kui's build.rs for the `kw_*` prototypes
and structs. There are few of either (Decision 2), which is the point.

### 2. The editor crosses as data: one door, the Lua names

The 262 names are not 262 prototypes. They are one:

```c
/* kawoosh.NAME(args...) with a C calling convention. `name` is the Lua
 * name ("buf.lines", "command", "spawn", "opt"); `args` is a list value,
 * one entry per Lua argument, borrowed for the call (you free it).
 * Answers the call's return value, yours to free, or NULL with the
 * reason in kw_error. A name this build has no door for is an error,
 * not a crash: probe for what you need in kw_ext_init. */
KuiValue *kw_call(KwCtx *ctx, KuiStr name, const KuiValue *args);
bool kw_error(KwCtx *ctx, KuiStr *out);         /* the last error, borrowed until the next call */
uint32_t kw_protocol(KwCtx *ctx);               /* the doors' version: kawoosh's release */
```

Why: the lua-boundary review found the API "grew a door per feature
more than a set of primitives", about 100 names called by one plugin
each. Mirroring that as prototypes is the third copy of the API the
cost estimate warned of, and an ABI bump per door. As data, a door is a
name and its arguments' shape — the same contract a Lua plugin reads in
`types/`, versioned by kawoosh's release through `kw_protocol`, never
by `KW_ABI_VERSION`. `KW_ABI_VERSION` moves when a `kw_*` prototype or
a `repr(C)` struct moves, which with a dozen of each is rare.

**How `kw_call` answers, round one: through the one Lua state.** The
arguments are converted to Lua values (kui-lua has the converter),
`kawoosh.NAME` is called, the result converted back. Every door exists
on day one, with the doc, the argument checks and the error messages
the Lua one has, and nothing in `lua/src/lib.rs` is written twice. The
conversion costs what a Lua plugin pays for the same call, and a
native extension's win was never the crossing — it is its own
computation, its threads (Decision 5) and its drawing (Decision 4).

**Later, per door, after a profile:** a door that shows up hot in a
native extension is re-expressed as a `Value`-native function in Rust
(`fn door(args: Value) -> Result<Value>`), which both bindings then
call — Lua through the same converter it uses today, C directly. The
protocol does not change; the Lua closure becomes one line. This is
the direction `lua/src/lib.rs` would benefit from regardless (one
7.5k-line file of closures), taken a door at a time, by measurement.

**Functions as arguments.** A `KuiValue` has no function variant; a
door that takes one (`command`, `on_*`, `spawn`'s `on_lines`,
`fs.apply`'s `done`, `timer`) takes a handle:

```c
typedef KuiValue *(*KwFn)(void *user, KwCtx *ctx, const KuiValue *args);
/* A callable value: put it where Lua would put a function. Called on
 * the UI thread with the arguments the door documents, borrowed for the
 * call; whatever it returns is freed by the host. Lives until kw_ext_free. */
KuiValue *kw_fn(KwCtx *ctx, KwFn fn, void *user);
```

The host keeps the pointer in a table and makes a Lua function that
calls it; a callback's arguments cross by the same conversion. A panic
inside a door never unwinds into C: every `kw_*` body is under
`catch_unwind` and answers an error. A C crash is a crash, as with a
grammar.

### 3. The context is good for the call

`KwCtx` is handed to `kw_ext_init`, to every `KwFn` and to the
`kui_ext_*` calls (read from the slot's params, Decision 4). It is
alive for that one call: a door called outside one answers an error.
Snapshot reads are consistent within a call; writes land after it, as a
Lua plugin's do (`wait_for_jobs` counts a `KwFn` that is running).

Ownership is one rule: **the extension frees what it made and what it
was returned; the host frees nothing of the extension's.** Arguments
and callback arguments are borrowed for the call; `KuiStr`s a `kw_*`
function answers (`kw_error`, Decision 6's `kw_buf_text`) are borrowed
until the next `kw_*` call on that context.

### 4. A native pane is a kui slot under the extension's namespace

A view registered through `kw_call("view", { NAME })` by an extension
loaded as namespace `NS` is drawn as `ui.slot_with("NS/NAME@PANE",
params)` — `Content::Lua(name)` grows the namespace it belongs to —
with the params a Lua view gets (`focused`, `width`, `height`,
`title_h`, `share`, `origin`, `prompt`). The extension's
`kui_ext_slots` answers `{ "*" }`, as the Lua extension's does, and its
`kui_ext_view` draws with `kui_open`/`kui_text`/`kui_close` on the
host's frame, reading `kui_slot_name` for which view and pane,
`kui_slot_params` for the rest, `kui_theme` for the colours. Clicks on
its own nodes come to `kui_ext_on_event` with the slot; keys are
kawoosh maps with `view = NAME`, registered through `kw_call("map",
…)`, so there is no key ABI. The kawoosh-drawn chrome a Lua view gets
from `boot.lua` — `ctx.field`, `ctx.keys`, `ctx.legend` — is Lua's; a
native view uses kui's `kui_text_input` and draws its own caps, or
ships a Lua half for the chrome and keeps the C half for the work.
`kui_ext_init` runs before `kw_ext_init` (kui's loader runs it), with
no `KwCtx`: state only, doors in `kw_ext_init`.

What this costs kawoosh: the namespace on `Content::Lua`, the slot
prefix, the params as they are, and routing a slot event under `NS/` to
the extension instead of `boot.lua` — which kui does already, since
nodes carry their origin. The drawing half of the ABI is kui's, built,
tested and versioned there.

### 5. One threading primitive: wake

Everything above runs on the UI thread. An extension that computes on a
thread of its own comes back with

```c
/* Runs `fn` on the UI thread with a fresh context, soon (the frame is
 * woken). Callable from any thread; the one kw_* function that is. */
void kw_wake(KwFn fn, void *user);
```

which is `kawoosh.spawn`'s `on_lines` without the process: the host's
channel to the frame (`Runtime::proc_lines` is the model). With it, a
native extension's heavy work is off the frame by construction, which
is more than a Lua plugin can say.

### 6. Typed fast paths, measured first

Two reads a native extension will ask for are big and the data route
copies them twice (snapshot → Lua string → `KuiStr`):

```c
/* The buffer's text, borrowed until the next kw_* call on `ctx`.
 * [lib]: the host owns it. One copy (the snapshot's), not two. */
bool kw_buf_text(KwCtx *ctx, uint64_t buffer, KuiStr *out);
/* Edits as an array, not a list value: { from, to, text } × n. [in]. */
bool kw_buf_edits(KwCtx *ctx, uint64_t buffer, const KwEdit *edits, size_t n);
```

Neither is in round one. They are added when a `lua_costs`-style row
shows the data route costing a frame (the review's rule: profile before
optimising), and each is one prototype and one struct, audited.

### 7. Loading, listing, trust

`kawoosh.extension("dupes", "~/.config/kawoosh/ext/dupes.so")` from
`init.lua`: the namespace is the first argument, the library the second
(`.so`, `.dylib`, `.dll` by platform, `.so` accepted on any, as
grammars are named). The app opens it as a `CExtension` (when it
exports `kui_ext_abi`) and reads the `kw_ext_*` symbols; a refusal
(no `kw_ext_abi`, another `KW_ABI_VERSION`, another `KUI_ABI_VERSION`,
a namespace taken, a path that leads nowhere) is a toast under the
`extension` source with the reason, and the namespace stays free.
`:extensions` is a list: namespace, name, path, both ABI numbers, the
doors it registered. Libraries are never unloaded (a `dlclose` with a
`kw_fn` handle alive is a dangling pointer; kui keeps its open too);
`:relaunch` picks up a rebuild. A project's `init.lua` may name one
under the trust it already needs to run at all (`trust.rs`): native
code is no more than the Lua that loads it, which is `kawoosh.spawn`
away from anything.

### 8. Windows: an import library the app ships

ELF and Mach-O resolve an extension's `kw_*` and `kui_*` from the
executable: the binary links with `-rdynamic` (one `rustc-link-arg`,
kui-ffi's build.rs is the model), the extension with `-undefined
dynamic_lookup` on macOS and nothing on Linux. Windows has no such
thing: a DLL names the module each import comes from. kawoosh.exe
exports both families through a `/DEF:` the build writes, and
`windows-app.nu` ships the `kawoosh.lib` link.exe produces beside it;
an extension links against that and loads into kawoosh and no other
host, which is the right answer for an editor's plugin. This is kui's
shape 2, chosen over shape 1 because kawoosh links kui-ffi statically
and ships no `kui_ffi.dll`.

### 9. Tested as kui tests its C half

- **A C extension built in the test.** `kawoosh/tests/ext/dupes.c` —
  the example below — compiled by the `cc` crate (in the graph) into
  the test's temp dir, loaded by the headless `Drive`, `:dupes` run on
  a buffer with duplicates, the scratch's lines asserted. The same
  plugin as `kawoosh/tests/ext/dupes.lua`, deliberately, with the two
  outputs asserted equal: the contract is the contract and the
  language is a detail.
- **The refusals**, each a case: no `kw_ext_abi`; `KW_ABI_VERSION` off
  by one; a door that does not exist; a door given the wrong shape (the
  Lua error, as a `kw_error` string); a `KwFn` that returns a value of
  the wrong shape; `kw_call` outside a call.
- **Parity**: `abi_parity` for the header, as kui's.
- **A native pane** drawn and clicked through `Drive`, its event
  reaching `kui_ext_on_event` and a reply reaching the host.
- **`kw_wake`** from a thread, landing on the next frame, counted by
  `wait_for_jobs`.

### 10. What a Rust author gets

Nothing extra in round one: a Rust extension is a `cdylib` with
`extern "C"` definitions of the `kw_ext_*` entry points and
declarations of the `kw_*`/`kui_*` functions, against the header. A
`kawoosh-ext` crate (safe wrappers over the two value types, a
`#[kawoosh::extension]` for the entry points, `Value` ↔ `KuiValue`)
is a round of its own when the first Rust extension asks; bindgen is
not wanted, the surface is small enough to write by hand and the crate
pins the ABI number it was written for.

## The example

`dupes.c`: the command `dupes` lists a buffer's duplicate lines in a
scratch pane. Nothing in it is drawn, so it exports the `kw_ext_*`
four only.

```c
#include "kawoosh.h"

uint32_t kw_ext_abi(void) { return KW_ABI_VERSION; }
const char *kw_ext_name(void) { return "dupes"; }

static KuiValue *run(void *user, KwCtx *ctx, const KuiValue *args) {
    (void)user; (void)args;                       /* args: the command's ctx map */
    KuiValue *none = kui_value_list();
    KuiValue *buf = kw_call(ctx, KUI_STR("buf.current"), none);
    KuiValue *lines = kw_call(ctx, KUI_STR("buf.lines"), none);   /* the current buffer's */
    kui_value_free(none);
    if (!buf || !lines) { kui_value_free(buf); kui_value_free(lines); return NULL; }

    /* ... one hash pass over kui_value_len(lines) × kui_value_at(lines, i) ... */
    KuiValue *out = kui_value_list();
    /* kui_value_list_push(out, kui_value_str(...)) per "L: same as F" */

    KuiValue *spec = kui_value_map();
    kui_value_map_set(spec, KUI_STR("name"), kui_value_str(KUI_STR("dupes")));
    kui_value_map_set(spec, KUI_STR("lines"), out);              /* consumed by the map */
    kui_value_map_set(spec, KUI_STR("read_only"), kui_value_bool(true));
    KuiValue *a = kui_value_list();
    kui_value_list_push(a, spec);
    KuiValue *r = kw_call(ctx, KUI_STR("buf.open_scratch"), a);
    kui_value_free(a); kui_value_free(r); kui_value_free(buf); kui_value_free(lines);
    return NULL;                                                 /* nothing to answer */
}

void *kw_ext_init(KwCtx *ctx) {
    KuiValue *a = kui_value_list();
    kui_value_list_push(a, kui_value_str(KUI_STR("dupes")));
    kui_value_list_push(a, kw_fn(ctx, run, NULL));
    KuiValue *r = kw_call(ctx, KUI_STR("command"), a);
    kui_value_free(a); kui_value_free(r);

    a = kui_value_list();
    kui_value_list_push(a, kui_value_str(KUI_STR("n")));
    kui_value_list_push(a, kui_value_str(KUI_STR("<leader>cd")));
    kui_value_list_push(a, kui_value_str(KUI_STR("dupes")));
    r = kw_call(ctx, KUI_STR("map"), a);
    kui_value_free(a); kui_value_free(r);
    return NULL;
}

void kw_ext_free(void *user) { (void)user; }
```

Built as a kui extension is built, linking against nothing:

```bash
cc -O2 -shared -undefined dynamic_lookup -I kawoosh/include -I kui/include dupes.c -o dupes.so
```

and loaded by one line in `init.lua`:

```lua
kawoosh.extension("dupes", "~/.config/kawoosh/ext/dupes.so")
```

The `KUI_STR`/`kui_value_*` ceremony is C's; the `kawoosh-ext` crate of
Decision 10 makes the same extension ten lines of Rust.

## What this buys over a Lua C module

The cheap option (a Lua C module through `package.loadlib`) crosses the
API at the same cost as this design's round one. What this one adds:
the Lua state stays safe (no `unsafe_new`); a native pane drawn through
kui's typed ABI, not through Lua tables; `kw_wake`, so native work is
off the frame; a refusal with a reason instead of a Lua error from
`require`; typed buffer access when measured (Decision 6); a Windows
story that does not need Lua in a DLL; and one library that is a kui
extension and a kawoosh extension at once.

## Estimate

Days are this project's rounds (a round a day, a merge a round). The
kui half — loader, values, strings, handshake, parity tooling, the
drawing ABI, Windows export shape — is built and is not counted; what
is counted is kawoosh's.

| round | what | days |
|---|---|---|
| 1 | `kawoosh.h` (four entry points, `kw_call`, `kw_fn`, `kw_error`, `kw_protocol`), the loader over `CExtension`, `catch_unwind` on every door, `kawoosh.extension`, `:extensions`, the refusals; `dupes.c` built by `cc` in a test and asserted equal to `dupes.lua` | 2 |
| 2 | Native panes: the namespace on `Content::Lua`, slot prefix and event routing, a drawn-and-clicked pane test; `kw_wake` with its `wait_for_jobs` row | 1–2 |
| 3 | `abi_parity` copied for `kw_*`; the `lua_costs` rows for a native extension's reads; `kw_buf_text` and `kw_buf_edits` if the rows say so; the first door made `Value`-native if one is hot | 1–2 |
| 4 | Windows: `/DEF:` export, `kawoosh.lib` shipped by `windows-app.nu`, a run there | 1, plus a Windows machine |
| 5 | `kawoosh-ext`, the Rust author's crate, when asked | 1–2 |

**Five to nine days** for rounds 1–4, two of them conditional on a
profile and a platform. Against the first estimate ("weeks, then a
permanent tax"): the weeks were a third copy of the API as prototypes,
which Decision 2 declines, and the drawing half, which kui carries. The
tax that remains is a `KW_ABI_VERSION` bump when one of about a dozen
prototypes or structs moves, and a parity test that says when the
header lied.

## Built

**Round one, 2026-10-08.** `lua/src/native.rs` (the loader, `kw_call`,
`kw_fn`, `kw_error`, `kw_protocol`, the two doors), `kawoosh/include/
kawoosh.h`, `kawoosh.extension` in `boot.lua`, `:extensions`
(`kawoosh/lua/extensions.lua`), the export flag in `kawoosh/build.rs`,
and `kawoosh/tests/native.rs` with `tests/ext/dupes.c`, `dupes.lua` and
`probe.c` — the C extension built by the system's `cc` in the test,
against kui's header found through `cargo metadata`, loaded, run, and
asserted equal to its Lua twin; every refusal with its reason; the
door's edges; the list. What the building decided beyond the draft:

- **The values cross through kui's C functions, in Rust too.**
  kui-ffi's `Value` is private to it (`KuiValue(pub(crate) Value)`),
  so the conversions to and from Lua walk `kui_value_at`,
  `kui_value_entry`, `kui_value_as_*` and build with `kui_value_map`,
  `kui_value_list_push` — the ABI itself, read from the host's side. A
  `From<Value>` in kui-ffi would make it a one-liner; not asked for.
- **A handle is a map with the one key `kw_fn`**, its value the index
  into the runtime's table of `(fn, user)`; the Lua function over it
  is made once and cached. The header says to build it with `kw_fn`,
  never by hand.
- **`kw_call` takes NULL for no arguments**, and a door answering
  several values answers a list of them (`nil, err` is a two-entry
  list, its first null). A scalar or a map where the list should be is
  an error, with the door's name.
- **A config reload runs `kw_ext_init` again.** The runtime is made
  anew then (`attach_lua`), its `Native` dropped with every extension's
  `kw_ext_free`, and the new `init.lua`'s `kawoosh.extension` opens the
  same library — never unloaded — and runs its init with a new context.
  The header says to keep state in what init returns, not in statics
  expected fresh.
- **The test binaries export too** (`rustc-link-arg-tests`), so a test
  can load an extension; `-Wl,-export_dynamic` on macOS, `--export-
  dynamic` elsewhere, nothing on Windows until round four.
- **`kawoosh.extension` is the boot script's**, over `kawoosh.
  _extension` in Rust: it expands the path (`~`, `..`) and makes a
  refusal a notification under the `extension` source as well as its
  `nil, err`.

## Open

- **A door's documentation for C.** `types/` is written for LuaLS. The
  same `meta.rs` can emit a `kawoosh-doors.md` naming each door's
  arguments as value shapes; until it does, the Lua types are the
  reference, and `kw_call` is the Lua call with the parentheses moved.
- **`kui_ext_init` before `kw_ext_init`**, with no context in the
  first. Fine for state; a plugin that wants a door in `kui_ext_init`
  waits a frame. If that bites, kawoosh's loader runs both itself and
  hands the context to both.
- **Unloading** is refused by design; a `kw_fn` handle may be alive. An
  `:extension reload NS` that refuses while any handle is registered is
  possible, and not worth the race until a developer loop asks.
