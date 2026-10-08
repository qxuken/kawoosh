/* The example of docs/design/native.md, as the tests build it: `:dupes`
 * lists a buffer's duplicate lines in a scratch pane. Nothing drawn, so
 * the four kw_ext_* only. `tests/ext/dupes.lua` is the same plugin in
 * Lua, deliberately: the contract is the contract and the language is a
 * detail. -DNO_ABI leaves kw_ext_abi out; -DABI=N claims another. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "kawoosh.h"

#ifndef NO_ABI
uint32_t kw_ext_abi(void) {
#ifdef ABI
    return ABI;
#else
    return KW_ABI_VERSION;
#endif
}
#endif

const char *kw_ext_name(void) { return "dupes"; }

typedef struct {
    const uint8_t *p;
    size_t n;
    size_t line; /* from 1; 0 = empty slot */
} Slot;

static uint64_t fnv(const uint8_t *p, size_t n) {
    uint64_t h = 1469598103934665603ull;
    for (size_t i = 0; i < n; i++) h = (h ^ p[i]) * 1099511628211ull;
    return h;
}

static void append(char **out, size_t *len, size_t *cap, const char *s, size_t n) {
    if (*len + n + 1 > *cap) {
        while (*len + n + 1 > *cap) *cap *= 2;
        *out = realloc(*out, *cap);
    }
    memcpy(*out + *len, s, n);
    *len += n;
}

static KuiValue *run(void *user, KwCtx *ctx, const KuiValue *args) {
    (void)user;
    (void)args;
    KuiValue *lines = kw_call(ctx, KUI_STR("buf.lines"), NULL);
    if (!lines) return NULL;
    size_t n = kui_value_len(lines);
    size_t cap = 16;
    while (cap < 2 * n + 2) cap <<= 1;
    Slot *tab = calloc(cap, sizeof *tab);
    size_t olen = 0, ocap = 256;
    char *out = malloc(ocap);
    for (size_t i = 0; i < n; i++) {
        KuiStr s;
        if (!kui_value_as_str(kui_value_at(lines, i), &s)) continue;
        size_t at = fnv(s.ptr, s.len) & (cap - 1);
        size_t first = 0;
        while (tab[at].line) {
            if (tab[at].n == s.len && memcmp(tab[at].p, s.ptr, s.len) == 0) {
                first = tab[at].line;
                break;
            }
            at = (at + 1) & (cap - 1);
        }
        if (first) {
            char row[64];
            int k = snprintf(row, sizeof row, "%zu: same as %zu\n", i + 1, first);
            append(&out, &olen, &ocap, row, (size_t)k);
        } else {
            tab[at].p = s.ptr;
            tab[at].n = s.len;
            tab[at].line = i + 1;
        }
    }
    if (olen == 0) {
        const char *none = "no duplicate lines";
        append(&out, &olen, &ocap, none, strlen(none));
    } else {
        olen--; /* the last newline */
    }
    KuiValue *spec = kui_value_map();
    kui_value_map_set(spec, KUI_STR("name"), kui_value_str(KUI_STR("dupes")));
    kui_value_map_set(spec, KUI_STR("text"), kui_value_str((KuiStr){(const uint8_t *)out, olen}));
    kui_value_map_set(spec, KUI_STR("read_only"), kui_value_bool(true));
    KuiValue *a = kui_value_list();
    kui_value_list_push(a, spec);
    KuiValue *r = kw_call(ctx, KUI_STR("buf.open_scratch"), a);
    kui_value_free(a);
    kui_value_free(r);
    kui_value_free(lines);
    free(tab);
    free(out);
    return NULL;
}

typedef struct {
    int inits;
} State;

void *kw_ext_init(KwCtx *ctx) {
    State *s = calloc(1, sizeof *s);
    s->inits++;
    KuiValue *a = kui_value_list();
    kui_value_list_push(a, kui_value_str(KUI_STR("dupes")));
    kui_value_list_push(a, kw_fn(ctx, run, s));
    KuiValue *r = kw_call(ctx, KUI_STR("command"), a);
    kui_value_free(a);
    kui_value_free(r);

    a = kui_value_list();
    kui_value_list_push(a, kui_value_str(KUI_STR("n")));
    kui_value_list_push(a, kui_value_str(KUI_STR("<leader>cd")));
    kui_value_list_push(a, kui_value_str(KUI_STR("dupes")));
    r = kw_call(ctx, KUI_STR("map"), a);
    kui_value_free(a);
    kui_value_free(r);
    return s;
}

void kw_ext_free(void *user) { free(user); }
