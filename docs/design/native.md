# Native extensions: a C ABI, the kui way

Status: asked 2026-10-08, "how expensive would be to open kawoosh for
native extension?", three ways costed (Lua C modules, `abi_stable`, an
ABI of kawoosh's own) and the third chosen: "option 3 is the way, let's
design, estimate this". The estimate is at the end. **Rounds one to
three built 2026-10-08**, "let's get building", "round two: native
panes and kw_wake", "round three: parity test and typed buffer
access"; **round four, Windows, 2026-10-09**, and the extension pack
the same day ("let's build pack.nu"); the blocks after the estimate say
what the building changed. Round five, a crate for Rust authors, waits
for the first to ask (backlog.md). The calls below are taken here, each the
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
namespace of its own; `kui.h` (4k lines; ABI 25 at the alpha.41
kawoosh builds with, 26 in kui's tree since) is the header, with
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

`kawoosh.extension("dupes")` from `init.lua`: the namespace is the
one argument, and the library is `ext/dupes.<ext>` under the config
directory, the extension the platform's (`.dylib`, `.so`, `.dll`; `.so`
accepted on any, as grammars are named) — one `init.lua` for every
machine, which a path spelling `~/.config` and `.so` was not. A second
argument says where instead: the library, a directory holding it, or
its path without the extension, as a grammar's `path` is read. For a
path spelled by hand, `kawoosh.fs.config()`, `fs.join` and
`fs.dylib(name)` name no platform. The app opens it as a `CExtension` (when it
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

`tests/ext/dupes.c`, as the tests build it: the command `dupes` lists
a buffer's duplicate lines in a scratch pane. Nothing drawn, so the
four `kw_ext_*` only; the hashing is left out here. The `kw_do`,
`kw_str`, `kw_map` spellings are the header's shorthand (static
inline, no ABI) over `kw_call` and kui's values.

```c
#include "kawoosh.h"

uint32_t kw_ext_abi(void) { return KW_ABI_VERSION; }
const char *kw_ext_name(void) { return "dupes"; }

static KuiValue *run(void *user, KwCtx *ctx, const KuiValue *args) {
    (void)user; (void)args;                         /* args[0]: the command's ctx map */
    KuiValue *lines = kw_call(ctx, KUI_STR("buf.lines"), NULL);   /* the current buffer's */
    if (!lines) return NULL;                        /* kw_error says why */
    size_t n = kui_value_len(lines);
    /* ... one hash pass over kui_value_at(lines, i), the report built into `out` ... */

    kw_do(ctx, "buf.open_scratch",
          kw_map("name", kw_str("dupes"), "text", kw_strn(out, olen), "read_only", kw_bool(true), NULL));
    kui_value_free(lines);
    return NULL;
}

void *kw_ext_init(KwCtx *ctx) {
    kw_do(ctx, "command", kw_str("dupes"), kw_fn(ctx, run, NULL));   /* kw_fn: where Lua takes a function */
    kw_do(ctx, "map", kw_str("n"), kw_str("<leader>cd"), kw_str("dupes"));
    return NULL;
}

void kw_ext_free(void *user) { (void)user; }
```

Built as a kui extension is built, linking against nothing:

```bash
cc -O2 -shared -undefined dynamic_lookup -I kawoosh/include -I kui/include dupes.c -o dupes.so
```

— on Windows against the import library the Kawoosh folder ships
(Decision 8), its headers beside it —

```bash
clang -O2 -shared -I Kawoosh\include dupes.c Kawoosh\kawoosh.lib -o dupes.dll
```

and loaded by one line in `init.lua`:

```lua
kawoosh.extension("dupes")   -- ext/dupes.<ext> under the config directory
```

A pane of its own is the same library with kui's seven entry points
beside these four (`tests/ext/panel.c`): `kw_ext_init` registers the
view as its own —

```c
KuiStr ns;
kw_namespace(ctx, &ns);
/* kawoosh.view("panel", nil, nil, { native = NS }) */
kw_do(ctx, "view", kw_str("panel"), kw_null(), kw_null(),
      kw_map("native", kw_strn(ns.ptr, ns.len), NULL));
```

— and `kui_ext_view` draws each pane of it with kui's own builders,
`kui_slot_params` saying which pane, how wide, whether focused:

```c
void kui_ext_view(void *user, KuiCtx *ui) {
    KuiTheme t = KUI_THEME_INIT;
    kui_theme(ui, &t);
    KuiSpec column = {.dir = KUI_COLUMN, .width = {KUI_GROW, 1}, .height = {KUI_GROW, 1}, .pad_l = 12};
    kui_open(ui, &column, NULL);
    KuiSpec row = {.dir = KUI_ROW};
    kui_open(ui, &row, kw_map("kind", kw_str("bump"), NULL));   /* its clicks come to kui_ext_on_event */
    KuiTextStyle style = {.size = 14, .color = t.fg};
    kui_text(ui, KUI_STR("a native pane"), &style);
    kui_close(ui);
    kui_close(ui);
}
```

A thread's end comes back with `kw_wake(fn, user)`, the one function
callable from any thread; `fn` runs on the UI thread with a context of
its own.

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
- **A path that names a platform is a bad connection string** (the
  user, on `"~/.config/kawoosh/ext/dupes.so"`): the library is found
  by convention from the namespace alone (`native::locate`, the
  grammars' rule), and `fs.config()` and `fs.dylib(name)` join
  `fs.join` as the helpers for a path spelled by hand; the config
  directory's rule moved to `kawoosh_systems::fs::config_dir`, one
  place for the settings, `init.lua`, the door and the lookup.

**Round two, 2026-10-08.** Native panes and `kw_wake`
(`tests/ext/panel.c`, both families in one library: a pane drawn by
its `kui_ext_view`, clicked, its count its own; a thread back through
`kw_wake`). What the building decided beyond the draft:

- **A view says its namespace.** `kawoosh.view(name, nil, nil, {
  native = NS })` registers a view whose pane is the slot
  `NS/name@PANE`; the extension reads `NS` with `kw_namespace`, a
  fifth function, since the host decides the namespace (kui's rule).
  The Lua function, when none is given, draws "`NS` draws no such
  view" — what a pane shows when the library has no `kui_ext_view`.
  `Content::Lua(name)` is unchanged: the namespace is looked up at
  draw, so maps with `view = name`, sessions and `:view_open` know
  nothing new.
- **`kw_ext_init` first, `kui_ext_init` on the first frame after.**
  The draft had kui's first. `kawoosh.extension` opens the library
  itself and runs `kw_ext_init` with a context there and then; a
  library that exports `kui_ext_abi` is noted (`draws` in
  `kawoosh.extensions()`) and the shell's next frame opens it again as
  a `CExtension` — the same handle, the loader's count up by one —
  and adds it under the namespace, where `Ui::add_extension` can run.
  So kui's init has no `KwCtx`, as the draft said, and the order is
  the one the two loaders allow: register in `kw_ext_init`, draw from
  `kui_ext_view`.
- **The kui half outlives a config reload.** It lives in the frame's
  extension list, outside the runtime; a reload runs `kw_ext_init`
  again and leaves the namespace kui holds as it is (a second
  `add_extension` would be refused). A refusal of the kui half — no
  `kui_ext_view`, another `KUI_ABI_VERSION` — is a toast under the
  `extension` source on that frame; the `kw` half is loaded regardless.
- **`kw_wake` is a static queue and the shell's wake handle**, set in
  `App::setup` (`native::set_waker`); before it is set, as in the
  tests, a wake queues and the next frame runs it. The frame runs the
  queue after the processes' lines (`sync_native`), `wait_for_jobs`
  runs it too, and a wake's context has no namespace — the header says
  to keep it in `user`. ABI 2.
- **The test drives the pane as a user would**: `:cpanel` opens it,
  the tab's slide is let run (`advance`), the row is clicked at its
  rect, the next frame's text carries the count; kui raised no warning.

**Round three, 2026-10-08.** The parity test, the cost rows and the
two typed doors (`tests/ext/typed.c`, `tests/ext/costs.c`; ABI 3).
What the building decided beyond the draft:

- **The parity test is kawoosh's own, in `tests/native.rs`**, kui's
  shape at kawoosh's size: a `repr(C)` struct is restated as a row
  whose destructuring pins every field to the Rust definition and
  whose offsets, sizes and alignment come from Rust; a `kw_*` function
  as a row that coerces the Rust function to the signature it spells
  and derives the C prototype from the Rust types (`c_of`), emitted as
  a typed function pointer that the header's prototype must be
  compatible with. The unit is compiled by the system's `cc` with
  `-fsyntax-only -Werror`, nothing linked; `KW_ABI_VERSION` is
  asserted equal to the Rust constant, and the header's set of `kw_*`
  prototypes is asserted equal to the rows' set both ways, so a
  function added to one side alone fails. No build.rs: eight
  functions and one struct do not need rows derived from source.
- **`kw_buf_text` answers one copy, borrowed until the call returns**
  — stronger than the draft's "until the next `kw_*` call", and
  simpler: the context keeps what it handed out. The data route is
  three copies (the snapshot's, the Lua string, the value).
- **`kw_buf_edits` is the Lua door's rule with no table between**:
  `check_edits` (a range ending before it starts, two that overlap)
  is one function both doors call, and the message is the same
  `Msg::Edits`. Carets are the Lua door's alone; an extension that
  wants them places them through `kw_call("buf.edits", …)`.
- **The typed doors read the runtime's snapshot and queue directly**:
  `Native` holds the same `Rc`s `seed` gives the Lua doors. A context
  has them from the one runtime; `0` is the current buffer.
- **Measured** (`tests/lua_costs.rs`, `native_buffer_access`, release,
  Apple Silicon, a 10 MB buffer, 10,000 edits):

  | route | C clock | wall around the command |
  |---|---|---|
  | `kw_call("buf.text")`, a read | 7.8 ms | 40 ms for 5 |
  | `kw_buf_text`, a read | 4.1 ms | 21 ms for 5 |
  | `kw_call("buf.edits")`, 10,000 edits queued | 5.3 ms | 8.1 ms with the apply |
  | `kw_buf_edits`, 10,000 edits queued | 0.16 ms | 1.9 ms with the apply |

  The read halves: what is left of the typed one is the snapshot's own
  copy out of the piece tree. The edits go thirty times faster to
  queue, and the engine's apply of ten thousand is under 2 ms either
  way — the crossing was the cost, as Decision 6 supposed.

**Reviewed before the push, 2026-10-08.** Four changes from the
review of the surface as rounds one to three left it, each applied:

- **`kw_error` is cleared** at the start of every `kw_call`, `kw_fn` and
  `kw_buf_*`: it is why the last of them answered NULL or false, never
  a stale reason after a success.
- **`kw_fn` deduplicates** on the function, the `user` and the
  namespace: a handle made per event or per spawn is one handle, not
  a table growing for the runtime's life.
- **A `kw_*` call off the UI thread answers NULL or false** and sets
  no error. The context carries the UI thread's id, copied out of the
  runtime when it is made, so the check touches nothing behind the
  `Rc`; before, such a call was undefined behaviour in the Lua state.
- **A door's `nil, why` is a refusal**: NULL and `why` in `kw_error`,
  the one shape every refusing door speaks on both sides, instead of a
  two-entry list whose first entry a reader had to test. A namespace
  is checked before the library is looked for, so the refusal names
  the namespace, not the places looked.

And three lines in the header: no `kw_*` function consumes a value
(unlike `kui_open` and `kui_value_map_set`); a buffer handle is never
0, which is why 0 can mean the current one; `kw_protocol` moves when a
door's shape changes after a release.

**And the call as a call** ("very luaish; i don't really like that we
use kui and kw in that strange way"): the ceremony was the caller
building Lua's argument list by hand, a push and a free per value,
and spelling kui's names on every line of an extension that never
draws. The fix is in the header alone — `static inline` over `kw_call`
and `kui_value_*`, since Rust cannot define a C variadic on stable and
the ABI should not carry what C can spell for itself: `kw_str`,
`kw_strn`, `kw_int`, `kw_bool`, `kw_float`, `kw_null` for the values
an extension makes; `kw_list(…)` and `kw_map("k", v, …, NULL)` for
the shapes; `kw_callv(ctx, "door", v, …)` and `kw_do(ctx, "door", v,
…)` for the call with its arguments consumed, the second dropping the
result and answering whether the door did; `kw_error_str` for a
message. A registration is one line. Reading what comes back stays
kui's (`kui_value_as_str`, `kui_value_at`): values are kui's as
`lua_State` is Lua's, and a second name for one type would be the
worse confusion. The parity test skips the inline lines, since they
are no ABI. Typed prototypes for `command`, `map` and `view` — the
third copy of the API, for three doors — stay declined until the
shorthand reads badly in a real extension.

**Round four, Windows, 2026-10-09.** Decision 8 built and run there
(Windows 11, x86_64, the MSVC target): `kawoosh.exe` exports every
`kw_*` and `kui_*`, `scripts/windows-app.nu` ships `kawoosh.lib` and an
`include\` with `kawoosh.h` and `kui.h`, and `tests/native.rs` runs on
Windows, its nine cases green. What the building decided beyond the
draft:

- **The export list is read from the headers**, as kui-ffi's build
  reads its sources: every prototype in `kawoosh.h` and in the graph's
  `kui.h` (found by `cargo metadata`, as the tests find it), the
  extension's own `*_ext_*` left out — 8 and 272, the same set kui-ffi
  defines. A name a header declares and nothing defines is link.exe's
  LNK2001 for the whole exe, never a quiet gap.
- **The `kawoosh` bin alone takes the `/DEF:`.** The other bins link no
  `kawoosh-lua`, and neither do three tests a `#![cfg(unix)]` empties on
  Windows, so a `/DEF:` for every test fails to link them. A test binary
  exports through `tests/drive.rs` instead: the same names as `/EXPORT:`
  linker directives in a `.drectve` section (`global_asm!`, the section
  a C compiler writes for a `dllexport`), so every test that can build
  an extension exports and one that cannot is untouched — the next
  `cfg(unix)` test is no trap.
- **A test's extension links against that test's own import library**
  (`native-<hash>.lib`, beside the binary): Windows has no
  `-undefined dynamic_lookup`, and the module an import names is the
  test binary. The tests build with `clang`, whose default target there
  is the MSVC ABI; `panel.c` starts its threads with Win32's where it
  had pthreads, and `costs.c` reads `QueryPerformanceCounter`.
- **Cargo leaves a bin's name unhashed on MSVC** (`deps/kawoosh.exe`),
  so the import library names `kawoosh.exe`, which is what runs; the
  `.lib` stays in `deps/`, where `windows-app.nu` takes it from.
- **Run there**: the release folder built (fat LTO keeps the exports;
  the shipped exe's table lists the 280), `dupes.c` built against the
  folder alone by clang and by a UCRT MinGW gcc, both loaded into the
  running app — one found by convention from `init.lua` as
  `ext\dupes.dll`, one by path under a second namespace — `:dupes` in
  each giving the same scratch, `:extensions` listing both; `panel.c`'s
  pane drawn in the window and `:cwake` back through `kw_wake`. Once
  the first `:cpanel` left the pane unopened; that was
  the driver: `kawoosh.exe` is a GUI-subsystem program, PowerShell's
  `&` does not wait for one, and two `kawoosh ex` started back to back
  raced to the socket, the command ahead of the load. Each waited on
  (`Start-Process -Wait`), the load and the open are in order every
  time, and the headless drive opens it with no frame between.
- **Two fixes the platform showed**: `locate` listed a path said with
  `.so` twice among the places looked (on every platform; the test
  asserted a prefix); `kawoosh.h`'s `kw_error_str` used `memcpy` with no
  `<string.h>`.
- **Measured on Windows** (`native_buffer_access`, release, a 10 MB
  buffer, 10,000 edits; the machine slower than the Apple Silicon one,
  the ratios the same):

  | route | C clock | wall around the command |
  |---|---|---|
  | `kw_call("buf.text")`, a read | 24.6 ms | 127 ms for 5 |
  | `kw_buf_text`, a read | 11.6 ms | 68 ms for 5 |
  | `kw_call("buf.edits")`, 10,000 edits queued | 27.6 ms | 42.8 ms with the apply |
  | `kw_buf_edits`, 10,000 edits queued | 0.70 ms | 8.7 ms with the apply |

**The extension pack, 2026-10-09** ("let's build pack.nu that
generates libs for major platforms and put headers. kui have something
like this"). `scripts/pack.nu` is kui's `pack-ffi.nu` at kawoosh's size:
`target/pack/kawoosh-ext-<version>-<platform>/` for darwin-arm64,
linux-x64, linux-arm64 and win32-x64, each with `include/` (`kawoosh.h`,
and `kui.h` from the kui-ffi the build links, LF whatever the checkout),
`example/dupes.c`, a `BUILD.txt` with the versions, both ABI numbers and
the platform's line that builds the example; a tarball each and
`SHA256SUMS`. What it decided:

- **Only Windows has a library.** On macOS and Linux an extension links
  against nothing, so there is nothing to ship but the headers;
  `BUILD.txt` says so rather than leaving `lib/` empty.
- **The import library is made from the list, not from a link.** It
  holds no code — the names and the module's, `kawoosh.exe` — so
  `llvm-dlltool` (or `zig dlltool`) makes it from `kawoosh-exports.def`,
  on any machine, for any platform: no container and no SDK, where kui's
  pack needs both to compile. `build.rs` now writes the list on every
  target, and the pack finds it through cargo's `build-script-executed`
  message, so the rule that reads the headers stays in one place.
- **The two import libraries agree**: the pack's and the one link.exe
  writes beside the exe name the same 280 and the same module, and
  import by name, so a DLL linked against either loads. Checked: the
  win32 tarball unpacked to a fresh folder, `dupes.c` built by its
  `BUILD.txt` line (and again against a `zig dlltool` library), both
  loaded into the release `kawoosh.exe` from `init.lua`, `:dupes` right.
- Windows on arm64 is a row of the table (`-m arm64`) when Kawoosh is
  built there.

## Open

- **A door's documentation for C.** `types/` is written for LuaLS. The
  same `meta.rs` can emit a `kawoosh-doors.md` naming each door's
  arguments as value shapes; until it does, the Lua types are the
  reference, and `kw_call` is the Lua call with the parentheses moved.
- **`kui_ext_init` after `kw_ext_init`**, with no context (round
  two). Fine for state; a plugin that wants a door from kui's side
  keeps a pointer to what `kw_ext_init` made. If that bites, kawoosh's
  loader runs both itself and hands the context to both.
- **Unloading** is refused by design; a `kw_fn` handle may be alive. An
  `:extension reload NS` that refuses while any handle is registered is
  possible, and not worth the race until a developer loop asks.
