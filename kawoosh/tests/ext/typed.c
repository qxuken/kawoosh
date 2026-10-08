/* The typed buffer doors (docs/design/native.md Decision 6): the text
 * read with one copy, edits applied from an array; each reported through
 * `echo`, the errors through kw_error. */
#include <stdio.h>
#include <string.h>

#include "kawoosh.h"

uint32_t kw_ext_abi(void) { return KW_ABI_VERSION; }

static void echo(KwCtx *ctx, const char *s) {
    KuiValue *a = kui_value_list();
    kui_value_list_push(a, kui_value_str(KUI_STR(s)));
    kui_value_free(kw_call(ctx, KUI_STR("echo"), a));
    kui_value_free(a);
}

static void echo_error(KwCtx *ctx, const char *head) {
    KuiStr e = KUI_STR("(no error)");
    kw_error(ctx, &e);
    char msg[256];
    snprintf(msg, sizeof msg, "%s%.*s", head, (int)e.len, (const char *)e.ptr);
    echo(ctx, msg);
}

static KuiValue *ctext(void *user, KwCtx *ctx, const KuiValue *args) {
    (void)user;
    (void)args;
    KuiStr s;
    if (!kw_buf_text(ctx, 0, &s)) {
        echo_error(ctx, "ctext: ");
        return NULL;
    }
    /* The string stays good across another door's call. */
    kui_value_free(kw_call(ctx, KUI_STR("buf.current"), NULL));
    char msg[64];
    snprintf(msg, sizeof msg, "typed: %zu bytes, head %.5s", s.len, (const char *)s.ptr);
    echo(ctx, msg);
    return NULL;
}

static KuiValue *ctext_bad(void *user, KwCtx *ctx, const KuiValue *args) {
    (void)user;
    (void)args;
    KuiStr s;
    if (kw_buf_text(ctx, 999999, &s)) echo(ctx, "ctext_bad: answered");
    else echo_error(ctx, "ctext_bad: ");
    return NULL;
}

static KuiValue *cedits(void *user, KwCtx *ctx, const KuiValue *args) {
    (void)user;
    (void)args;
    KwEdit edits[] = {
        {0, 0, KUI_STR("X")},
        {2, 3, KUI_STR("YY")},
    };
    if (kw_buf_edits(ctx, 0, edits, 2)) echo(ctx, "cedits: applied");
    else echo_error(ctx, "cedits: ");
    return NULL;
}

static KuiValue *cedits_bad(void *user, KwCtx *ctx, const KuiValue *args) {
    (void)user;
    (void)args;
    KwEdit overlap[] = {{0, 2, KUI_STR("a")}, {1, 3, KUI_STR("b")}};
    char msg[256];
    if (kw_buf_edits(ctx, 0, overlap, 2)) {
        echo(ctx, "cedits_bad: overlap applied");
        return NULL;
    }
    KuiStr e;
    kw_error(ctx, &e);
    int n = snprintf(msg, sizeof msg, "%.*s | ", (int)e.len, (const char *)e.ptr);
    KwEdit backwards[] = {{3, 1, KUI_STR("c")}};
    if (kw_buf_edits(ctx, 0, backwards, 1)) {
        echo(ctx, "cedits_bad: backwards applied");
        return NULL;
    }
    kw_error(ctx, &e);
    snprintf(msg + n, sizeof msg - (size_t)n, "%.*s", (int)e.len, (const char *)e.ptr);
    echo(ctx, msg);
    return NULL;
}

static void command(KwCtx *ctx, const char *name, KwFn fn) {
    KuiValue *a = kui_value_list();
    kui_value_list_push(a, kui_value_str(KUI_STR(name)));
    kui_value_list_push(a, kw_fn(ctx, fn, NULL));
    kui_value_free(kw_call(ctx, KUI_STR("command"), a));
    kui_value_free(a);
}

void *kw_ext_init(KwCtx *ctx) {
    command(ctx, "ctext", ctext);
    command(ctx, "ctext_bad", ctext_bad);
    command(ctx, "cedits", cedits);
    command(ctx, "cedits_bad", cedits_bad);
    return NULL;
}
