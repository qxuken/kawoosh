#!/usr/bin/env nu

const ROOT = path self | path dirname | path dirname

# Explicit source update, shared by language tools and grammars. Packaging
# never downloads a moving catalog implicitly.
def main [kind: string, --release: string, --url: string] {
  if $kind not-in [tools grammars] { error make { msg: "kind must be tools or grammars" } }
  if $release == null { error make { msg: "name the release with --release rN" } }
  let repo = $"qxuken/kawoosh-($kind)"
  let source_url = if $url == null { $"https://github.com/($repo)/releases/download/($release)/manifest.json" } else { $url }
  let folder = $ROOT | path join kawoosh $kind
  let pending = $folder | path join $".manifest-(random uuid).json"
  http get --raw --max-time 30sec $source_url | save --raw $pending
  ^nu ($ROOT | path join scripts verify-catalog.nu) $kind --path $pending
  if $kind == tools and (open $pending | get release) != $release { error make { msg: "manifest and requested release disagree" } }
  let digest = open --raw $pending | hash sha256
  mv --force $pending ($folder | path join manifest.json)
  { repository: $repo, release: $release, url: $source_url, sha256: $digest } | to json | save --force ($folder | path join source.json)
  print $"updated ($kind) snapshot to ($release); review its diff and run the repository checks"
}
