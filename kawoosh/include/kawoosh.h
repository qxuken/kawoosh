/* kawoosh.h - the native extension ABI (docs/design/native.md).
 *
 * An extension is a shared library that `init.lua` names:
 *
 *     kawoosh.extension("dupes")
 *
 * which is `ext/dupes.<ext>` under the config directory, the extension
 * the platform's (.dylib, .so, .dll; .so accepted anywhere) - one
 * init.lua for every machine. A second argument says where instead: the
 * library, a directory holding it, or its path without the extension;
 * `kawoosh.fs.config()`, `fs.join` and `fs.dylib(name)` spell a path by
 * hand without naming a platform.
 *
 * It links against nothing. Every kw_* and kui_* it calls is left
 * undefined and resolved from the kawoosh executable at load, the way a
 * Lua C module resolves lua_* (on Windows, from the import library the
 * app ships; see native.md Decision 8). Build it as a kui extension is
 * built:
 *
 *     cc -O2 -shared -undefined dynamic_lookup -I kawoosh/include -I kui/include dupes.c -o dupes.so
 *
 * on macOS; `-shared -fPIC` and no `-undefined` on Linux.
 *
 * YOU define the four kw_ext_* below; kawoosh calls them. Two are
 * required - kw_ext_abi and, in practice, kw_ext_init, since an
 * extension that registers nothing does nothing. The strings and values
 * are kui's (KuiStr, KuiValue, the kui_value_* functions): the one value
 * type an extension already builds for kui_open. An extension that also
 * defines kui's kui_ext_* entry points draws its own panes (native.md
 * Decision 4); one that defines these four only is a command-line
 * extension.
 *
 * A PANE OF YOUR OWN. Register a view whose drawing is yours:
 *
 *     KuiStr ns; kw_namespace(ctx, &ns);
 *     kw_call(ctx, "view", [ "panel", null, null, { native = ns } ])
 *
 * and define kui's seven (kui_ext_abi returning KUI_ABI_VERSION,
 * kui_ext_slots answering { "*" }, kui_ext_view, kui_ext_on_event and
 * the rest). `:view_open panel`, or kw_call("view_open", ["panel"]),
 * puts it in a pane; each pane is the slot NAMESPACE/panel@PANE, which
 * your kui_ext_view fills with kui_open / kui_text / kui_close, reading
 * kui_slot_params for `pane`, `focused`, `width`, `height`, `title_h`
 * and kui_theme for the colours. Clicks on your own nodes come to
 * kui_ext_on_event; keys are kawoosh maps with `view = "panel"`,
 * registered through kw_call("map", ...). Your kui_ext_init runs on the
 * first frame after kw_ext_init, with no KwCtx and a state of its own:
 * register in kw_ext_init, draw from kui_ext_view.
 *
 * A THREAD OF YOUR OWN. Everything here runs on the UI thread. Work you
 * do on a thread comes back with kw_wake(fn, user), the one function
 * callable from any thread: fn runs on the UI thread, soon, with a
 * context of its own.
 *
 * Ownership is one rule: the extension frees what it made and what it
 * was returned; the host frees nothing of the extension's. Arguments
 * are borrowed for the call. A KuiStr a kw_* function answers is
 * borrowed until the next kw_* call on that context.
 *
 * Everything runs on the UI thread, inside kw_ext_init or a KwFn the
 * host is calling. The KwCtx handed to either is alive for that one
 * call: store nothing.
 */
#ifndef KAWOOSH_H
#define KAWOOSH_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "kui.h"

#ifdef __cplusplus
extern "C" {
#endif

/* The ABI this header describes. kw_ext_abi answers it; a build whose
 * number differs refuses the extension at load, with the two numbers.
 * It moves when a prototype or struct in this file moves - rarely, since
 * the editor crosses as data (kw_call) and not as prototypes. Which
 * doors exist, and what they take, is the protocol (kw_protocol) and
 * moves with kawoosh's releases. */
#define KW_ABI_VERSION 2

/* One call's context: the runtime's Lua and its native state. Opaque. */
typedef struct KwCtx KwCtx;

/* A function the extension hands over through kw_fn: called on the UI
 * thread with the arguments the door documents, as a list borrowed for
 * the call (NULL for none); what it returns is the host's to free, or
 * NULL for nothing. `user` is what kw_fn was given. */
typedef KuiValue *(*KwFn)(void *user, KwCtx *ctx, const KuiValue *args);

#define KW_EXT_EXPORT KUI_EXT_EXPORT

/* -- Your side: the four entry points --------------------------------- */

/* REQUIRED. Your KW_ABI_VERSION. Absent is refused as a mismatch is: an
 * extension built against a header from before the symbol existed is
 * exactly the mismatched extension the check is for. */
KW_EXT_EXPORT uint32_t kw_ext_abi(void);

/* A name for `:extensions` and the log; a NUL-terminated static string.
 * Absent = the library's file stem. */
KW_EXT_EXPORT const char *kw_ext_name(void);

/* Where you register what you are: commands, maps, views, hooks, through
 * kw_call. Whatever you return is your state, handed to kw_ext_free.
 * Runs again, with a new context, when the Lua runtime is made anew (a
 * config reload) - the library is never unloaded, so keep your state in
 * what you return rather than in statics you expect fresh. */
KW_EXT_EXPORT void *kw_ext_init(KwCtx *ctx);

/* Your state, when the runtime that loaded you goes. */
KW_EXT_EXPORT void kw_ext_free(void *user);

/* -- The library's side: what an extension calls ---------------------- */

/* kawoosh.NAME(args...) with a C calling convention. `name` is the Lua
 * name ("buf.lines", "command", "map", "spawn", "opt"); `args` is a list
 * value, one entry per Lua argument, or NULL for none. Answers the
 * call's return value - a list of them when the door returns several
 * (`nil, err` is a two-entry list) - yours to free, or NULL with the
 * reason in kw_error: a name this build has no door for, arguments that
 * are not a list, or the Lua error the door raised. A function in a
 * result is null; a table with keys 1..n is a list, any other a map. */
KuiValue *kw_call(KwCtx *ctx, KuiStr name, const KuiValue *args);

/* The last error a kw_* call on this context left, borrowed until the
 * next; false with nothing written when there is none. */
bool kw_error(KwCtx *ctx, KuiStr *out);

/* The doors' version; 0 on a null context. */
uint32_t kw_protocol(KwCtx *ctx);

/* The namespace the extension whose call this is was loaded under -
 * what a view registers as `native`, and what tells one instance of you
 * from another when a host loads you twice. Borrowed for the call.
 * False with nothing written on a wake's context (kw_wake) or a null
 * one. */
bool kw_namespace(KwCtx *ctx, KuiStr *out);

/* fn(user, ctx, NULL) on the UI thread, soon: the frame is woken. The
 * one kw_* callable from any thread. The context it gets has no
 * namespace; keep yours in `user`. A null fn does nothing. */
void kw_wake(KwFn fn, void *user);

/* A callable value, to put where Lua would put a function: a command's
 * body, an on_* hook, a spawn's on_lines. Yours to free like any value,
 * and consumed like any value when a door takes it; the function it
 * names lives until the runtime goes. NULL for a null `fn`. The value is
 * a map with the one key "kw_fn"; build it here, not by hand. */
KuiValue *kw_fn(KwCtx *ctx, KwFn fn, void *user);

#ifdef __cplusplus
}
#endif

#endif /* KAWOOSH_H */
