/* The typed buffer doors (docs/design/native.md Decision 6): the text
 * read with one copy, edits applied from an array; each reported through
 * `echo`, the errors through kw_error. */
#include <stdio.h>
#include <string.h>

#include "kawoosh.h"

uint32_t kw_ext_abi(void) { return KW_ABI_VERSION; }

static void echo(KwCtx *ctx, const char *s) { kw_do(ctx, "echo", kw_str(s)); }

static void echo_error(KwCtx *ctx, const char *head) {
    char why[200], msg[256];
    snprintf(msg, sizeof msg, "%s%s", head, kw_error_str(ctx, why, sizeof why));
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

void *kw_ext_init(KwCtx *ctx) {
    kw_do(ctx, "command", kw_str("ctext"), kw_fn(ctx, ctext, NULL));
    kw_do(ctx, "command", kw_str("ctext_bad"), kw_fn(ctx, ctext_bad, NULL));
    kw_do(ctx, "command", kw_str("cedits"), kw_fn(ctx, cedits, NULL));
    kw_do(ctx, "command", kw_str("cedits_bad"), kw_fn(ctx, cedits_bad, NULL));
    return NULL;
}
