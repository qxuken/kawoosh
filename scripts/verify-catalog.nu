#!/usr/bin/env nu

const ROOT = path self | path dirname | path dirname

# Verify either independently released catalog without executing its recipes.
def main [kind: string, --path: path] {
  if $kind not-in [tools grammars] { error make { msg: "kind must be tools or grammars" } }
  let folder = $ROOT | path join kawoosh $kind
  let file = if $path == null { $folder | path join manifest.json } else { $path }
  let catalog = open $file
  if $kind == tools {
    if $catalog.schema != 1 { error make { msg: "unsupported tools schema" } }
    if ($catalog.release | str trim | is-empty) { error make { msg: "missing tools release" } }
    for group in [servers formatters] {
      let rows = $catalog | get $group
      if ($rows | is-empty) { error make { msg: $"empty ($group) catalog" } }
      let names = $rows | get name
      if ($names | uniq | length) != ($names | length) { error make { msg: $"duplicate ($group) recipe" } }
      for row in $rows {
        if not ($row.name =~ '^[a-zA-Z0-9_.-]+$') or ($row.cmd | str trim | is-empty) { error make { msg: "invalid recipe name or command" } }
        for field in [args languages roots with] {
          let value = $row | get -o $field
          if $value != null {
            if not (($value | describe) | str starts-with list) or not ($value | all {|v| ($v | describe) == string }) { error make { msg: $"($row.name).($field) must be a list of strings" } }
          }
        }
      }
    }
  } else {
    if $catalog.format != 1 { error make { msg: "unsupported grammar format" } }
    if ($catalog.targets | is-empty) or ($catalog.grammars | is-empty) { error make { msg: "empty grammar catalog" } }
    for row in ($catalog.grammars | transpose name recipe) {
      if ($row.recipe.archive | is-empty) or ($row.recipe.blake3 | str length) != 64 or $row.recipe.size <= 0 { error make { msg: $"invalid grammar archive: ($row.name)" } }
    }
  }
  let source = $folder | path join source.json
  if $path == null and ($source | path exists) {
    let pinned = open $source
    if (open --raw $file | hash sha256) != $pinned.sha256 { error make { msg: $"($kind) snapshot differs from its recorded release digest" } }
  }
  print $"verified ($kind) catalog"
}
