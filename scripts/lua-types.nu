#!/usr/bin/env nu
# The Lua API's types checked by lua-language-server itself
# (docs/design/lua-types.md Decision 6): the meta files as a launch
# writes them — `kawoosh/src/types.rs`'s ratchet test writes them where
# `KAWOOSH_META_OUT` says — in a workspace of their own beside
# `lua-types/sample.lua`, a plugin's worth of typed calls, and
# `lua-language-server --check` over it all. What it finds must be
# exactly the sample's `-- expect: CODE` lines: anything else — in the
# meta (a malformed annotation, a class used and never declared) or in
# the sample (a typed call flagged) — fails, and so does an expected
# error the server missed.

# Check the generated Lua types with lua-language-server.
def main [
  --keep  # leave the workspace in place and print where
] {
  if (which lua-language-server | is-empty) {
    error make { msg: "lua-language-server is not on the PATH (brew install lua-language-server)" }
  }
  cd ($env.FILE_PWD | path dirname)
  # Not under a path with `claude` in it: 3.19's `--check` mangles one.
  let tmp = (mktemp -d -t kawoosh-lua-types.XXXXXX)
  let ws = ($tmp | path join ws)
  let meta = ($ws | path join meta)
  mkdir $meta
  print "== meta"
  # The ratchet may fail and still have written the files: its list is
  # printed, the check goes on.
  try {
    with-env { KAWOOSH_META_OUT: $meta } {
      cargo test -p kawoosh --lib types::tests::the_api_is_typed -- --exact --nocapture
    }
  }
  if not ($meta | path join kawoosh.lua | path exists) {
    error make { msg: "the meta was not written" }
  }
  cp scripts/lua-types/sample.lua $ws
  { "runtime.version": "Lua 5.5", "diagnostics.globals": [] } | to json | save ($ws | path join .luarc.json)
  print "== lua-language-server --check"
  let out = ($tmp | path join check.json)
  (lua-language-server $"--check=($ws)" --checklevel=Warning --check_format=json
    $"--check_out_path=($out)" $"--logpath=($tmp | path join log)"
    $"--metapath=($tmp | path join metapath)") | ignore
  let found = if ($out | path exists) {
    open $out | transpose uri diags | each {|f|
      $f.diags | each {|d| {
        file: ($f.uri | path basename)
        line: ($d.range.start.line + 1)
        code: $d.code
        message: $d.message
      } }
    } | flatten
  } else { [] }
  let expected = (open scripts/lua-types/sample.lua | lines | enumerate
    | where {|l| $l.item =~ '-- expect: ' }
    | each {|l| { file: "sample.lua", line: ($l.index + 1), code: ($l.item | parse -r '-- expect: (?<c>[\w-]+)' | get c.0) } })
  let unexpected = ($found | where {|d|
    not ($expected | any {|e| $e.file == $d.file and $e.line == $d.line and $e.code == $d.code })
  })
  let missed = ($expected | where {|e|
    not ($found | any {|d| $e.file == $d.file and $e.line == $d.line and $e.code == $d.code })
  })
  if $keep { print $"workspace: ($ws)" } else { rm -rf $tmp }
  if ($unexpected | is-not-empty) {
    print "== found and not expected"
    print ($unexpected | table -e)
  }
  if ($missed | is-not-empty) {
    print "== expected and not found"
    print ($missed | table -e)
  }
  if ($unexpected | is-not-empty) or ($missed | is-not-empty) {
    error make { msg: "the Lua types do not check" }
  }
  print $"== checked: ($expected | length) expected errors found, nothing else"
}
