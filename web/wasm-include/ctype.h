#ifndef TREE_SITTER_WASM_CTYPE_H_
#define TREE_SITTER_WASM_CTYPE_H_

typedef void *locale_t;

#ifndef weak_alias
#define weak_alias(old, new) \
  extern __typeof(old) new __attribute__((__weak__, __alias__(#old)))
#endif

int isblank(int c);

/* Added for kawoosh's browser build (web/README.md): tree-sitter-md's
   scanner calls isdigit, which this header did not declare. The rest of
   the classic set with it; wasi-libc, linked into the module, defines
   them all. */
int isalnum(int c);
int isalpha(int c);
int iscntrl(int c);
int isdigit(int c);
int isgraph(int c);
int islower(int c);
int ispunct(int c);
int isspace(int c);
int isupper(int c);
int isxdigit(int c);
int tolower(int c);
int toupper(int c);

static inline int isprint(int c) {
  return c >= 0x20 && c <= 0x7E;
}

#endif // TREE_SITTER_WASM_CTYPE_H_
