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
 * The extension pack for each platform (kawoosh-ext-<version>-<platform>,
 * made by scripts/pack.nu) holds this header, kui.h at the kui this
 * Kawoosh is built with, an example, and on Windows kawoosh.lib; its
 * BUILD.txt has the line below for that platform.
 *
 * It links against nothing. Every kw_* and kui_* it calls is left
 * undefined and resolved from the kawoosh executable at load, the way a
 * Lua C module resolves lua_*. Build it as a kui extension is built:
 *
 *     cc -O2 -shared -undefined dynamic_lookup -I kawoosh/include -I kui/include dupes.c -o dupes.so
 *
 * on macOS; `-shared -fPIC` and no `-undefined` on Linux.
 *
 * On Windows a DLL may leave nothing undefined: it names the module each
 * import comes from, and takes that name from an import library. The
 * Kawoosh folder ships kawoosh.exe's, kawoosh.lib, with this header and
 * kui.h in its include folder, and so does the win32 pack (native.md
 * Decision 8):
 *
 *     clang -O2 -shared -I Kawoosh\include dupes.c Kawoosh\kawoosh.lib -o dupes.dll
 *
 * clang-cl and a UCRT MinGW gcc build it the same. Linked so, it loads
 * into that kawoosh.exe and no other host.
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
 *     kw_do(ctx, "view", kw_str("panel"), kw_null(), kw_null(),
 *           kw_map("native", kw_strn(ns.ptr, ns.len), NULL));
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
 * A pane that did not change is not drawn again: kawoosh tells kui so
 * and kui pushes the last fill's nodes without calling kui_ext_view. It
 * knows nothing changed when kawoosh has called none of your code since
 * - no KwFn (a command, a map, a hook, a wake), no kui_ext_on_event;
 * kui checks the rest of what you read through it (the params, the
 * theme, a hover, a scroll offset, kui_now). So change
 * what your view draws only inside one of those calls - from a thread,
 * through kw_wake - never in kui_ext_view itself, and read the time
 * through kui_now, not a clock of your own.
 *
 * A THREAD OF YOUR OWN. Everything here runs on the UI thread. Work you
 * do on a thread comes back with kw_wake(fn, user), the one function
 * callable from any thread: fn runs on the UI thread, soon, with a
 * context of its own.
 *
 * Ownership is one rule: the extension frees what it made and what it
 * was returned; the host frees nothing of the extension's. Arguments
 * are borrowed for the call - no kw_* function consumes a value, unlike
 * kui_open or kui_value_map_set. A KuiStr a kw_* function answers is
 * borrowed until the call that asked returns.
 *
 * Errors are strings: a kw_* that answers NULL or false left the reason
 * in kw_error, which the next kw_call, kw_fn or kw_buf_* on the context
 * clears or sets anew.
 *
 * Everything runs on the UI thread, inside kw_ext_init or a KwFn the
 * host is calling; a thread of yours comes back through kw_wake. A kw_*
 * call from any other thread answers NULL or false and sets no error,
 * since nothing on that thread may touch the editor. The KwCtx handed
 * to a call is alive for that one call: store nothing.
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
#define KW_ABI_VERSION 3

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
 * call's return value - a list of them when the door returns several -
 * yours to free, or NULL with the reason in kw_error: a name this build
 * has no door for, arguments that are not a list, the Lua error the
 * door raised, or the door's own refusal (a door that answers `nil, why`
 * has refused; `why` is the reason). A function in a result is null; a
 * table with keys 1..n is a list, any other a map. */
KuiValue *kw_call(KwCtx *ctx, KuiStr name, const KuiValue *args);

/* Why the last kw_call, kw_fn or kw_buf_* on this context answered NULL
 * or false, borrowed until the next of them; false with nothing written
 * when it succeeded. */
bool kw_error(KwCtx *ctx, KuiStr *out);

/* The doors' version: which kawoosh.* names exist and what they take.
 * It moves when a door's shape changes after a release; 0 on a null
 * context. */
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

/* -- Typed buffer access: the two reads the data route copies twice --
 *
 * kw_call("buf.text") copies the text three times (the snapshot's, the
 * Lua string, the value) and kw_call("buf.edits") makes a Lua table per
 * edit. These do the same work with one copy and no table; measured in
 * tests/lua_costs.rs, `native_buffer_access`. Everything else stays
 * kw_call. `buffer` is a handle from kw_call("buf.current") or
 * ("buf.list") - never 0 - or 0 for the current one. */

/* The buffer's text, one copy, [lib]: borrowed until the call that
 * asked returns. False with the reason in kw_error. */
bool kw_buf_text(KwCtx *ctx, uint64_t buffer, KuiStr *out);

/* One edit: bytes from..to (to exclusive) replaced by text. [in]: yours,
 * read during the call. */
typedef struct KwEdit {
    uint64_t from;
    uint64_t to;
    KuiStr text;
} KwEdit;

/* n edits applied at once, each range in the text before them, none
 * overlapping - what kw_call("buf.edits", ...) does from a list. False
 * with the reason in kw_error: a range ending before it starts, two
 * that overlap, a buffer that is not. */
bool kw_buf_edits(KwCtx *ctx, uint64_t buffer, const KwEdit *edits, size_t n);

/* A callable value, to put where Lua would put a function: a command's
 * body, an on_* hook, a spawn's on_lines. Yours to free like any value,
 * and consumed like any value when a door takes it; the function it
 * names lives until the runtime goes. The same fn with the same user is
 * the same handle, so one made per event or per spawn costs nothing
 * after the first. NULL for a null `fn`. The value is a map with the
 * one key "kw_fn"; build it here, not by hand. */
KuiValue *kw_fn(KwCtx *ctx, KwFn fn, void *user);

/* -- Shorthand: the call as a call -------------------------------------
 *
 * Nothing below is ABI: static inline over kw_call and kui_value_*, in
 * this header alone, so the parity test and the ABI number do not know
 * it. Values are kui's, as lua_State is Lua's; these spell the ones
 * you make from C strings and literals, and let a door be called in one
 * line with its arguments consumed, so there is nothing to free but
 * what comes back:
 *
 *     kw_do(ctx, "command", kw_str("dupes"), kw_fn(ctx, run, s));
 *     kw_do(ctx, "map", kw_str("n"), kw_str("<leader>cd"), kw_str("dupes"));
 *     KuiValue *lines = kw_callv(ctx, "buf.lines", kw_int(buffer));
 *
 * kw_callv and kw_do take one argument at least; a door called with
 * none is kw_call(ctx, KUI_STR(name), NULL). Reading what comes back
 * stays kui_value_as_str, kui_value_at and the rest. */

#include <stdarg.h>
#include <string.h>

static inline KuiValue *kw_null(void) { return kui_value_null(); }
static inline KuiValue *kw_bool(bool v) { return kui_value_bool(v); }
static inline KuiValue *kw_int(int64_t v) { return kui_value_int(v); }
static inline KuiValue *kw_float(double v) { return kui_value_float(v); }
/* A NUL-terminated C string, copied. */
static inline KuiValue *kw_str(const char *s) { return kui_value_str(KUI_STR(s)); }
/* `n` bytes at `p`, copied - a KuiStr's, a buffer's. */
static inline KuiValue *kw_strn(const void *p, size_t n) {
    return kui_value_str((KuiStr){(const uint8_t *)p, n});
}

/* kw_list(v, ...): a list of the values, consumed; kw_map("k", v, ...,
 * NULL): a map of the pairs, consumed. Both end on NULL, which the
 * macros supply for the list and you supply for the map. */
static inline KuiValue *kw_list_(KuiValue *first, ...) {
    KuiValue *list = kui_value_list();
    va_list ap;
    va_start(ap, first);
    for (KuiValue *v = first; v; v = va_arg(ap, KuiValue *)) kui_value_list_push(list, v);
    va_end(ap);
    return list;
}
#define kw_list(...) kw_list_(__VA_ARGS__, NULL)

static inline KuiValue *kw_map(const char *key, ...) {
    KuiValue *map = kui_value_map();
    va_list ap;
    va_start(ap, key);
    for (const char *k = key; k; k = va_arg(ap, const char *))
        kui_value_map_set(map, KUI_STR(k), va_arg(ap, KuiValue *));
    va_end(ap);
    return map;
}

/* kw_callv(ctx, "door", v, ...): kw_call with the values as its
 * arguments, consumed; what comes back is yours, NULL with kw_error as
 * kw_call's. kw_do: the same, the result dropped, true when the door
 * answered. */
static inline KuiValue *kw_callv_(KwCtx *ctx, KuiStr name, KuiValue *first, ...) {
    KuiValue *args = kui_value_list();
    va_list ap;
    va_start(ap, first);
    for (KuiValue *v = first; v; v = va_arg(ap, KuiValue *)) kui_value_list_push(args, v);
    va_end(ap);
    KuiValue *out = kw_call(ctx, name, args);
    kui_value_free(args);
    return out;
}
#define kw_callv(ctx, name, ...) kw_callv_((ctx), KUI_STR(name), __VA_ARGS__, NULL)

static inline bool kw_do_(KwCtx *ctx, KuiStr name, KuiValue *first, ...) {
    KuiValue *args = kui_value_list();
    va_list ap;
    va_start(ap, first);
    for (KuiValue *v = first; v; v = va_arg(ap, KuiValue *)) kui_value_list_push(args, v);
    va_end(ap);
    KuiValue *out = kw_call(ctx, name, args);
    kui_value_free(args);
    bool ok = out != NULL;
    kui_value_free(out);
    return ok;
}
#define kw_do(ctx, name, ...) kw_do_((ctx), KUI_STR(name), __VA_ARGS__, NULL)

/* The reason the last call failed, as a C string into `buf`: for a
 * message. Empty when there was none. */
static inline const char *kw_error_str(KwCtx *ctx, char *buf, size_t cap) {
    KuiStr e = {0, 0};
    size_t n = kw_error(ctx, &e) && e.len < cap ? e.len : (cap ? cap - 1 : 0);
    if (cap) {
        if (n) memcpy(buf, e.ptr, n);
        buf[n] = 0;
    }
    return buf;
}

#ifdef __cplusplus
}
#endif

#endif /* KAWOOSH_H */
