-- The memory's ranking (memory.md Decision 8): `kawoosh.memory_rank`
-- is the module, and `memory.rank(row, now)` the score a subject row
-- gets for a list — the same formula the engine evicts by, as its
-- default, and yours to replace from a config: which file should come
-- first for *you* is exactly what "hackable by design" says a user
-- rewrites. The eviction stays the engine's, fixed, whatever `rank`
-- says. `memory.boosts(kind, limit)` turns the rows into the picker's
-- boosts — a path to a number in (0, 0.5], a pinned row above any
-- score — which `picker.lua` reads for its files, buffers and smart
-- sources.
local memory = { half_life_days = 7 }
kawoosh.memory_rank = memory

-- memory.rank(row, now): the row's signals — visits, three per edit,
-- two per yank, its dwell in minutes up to sixty — halved for every
-- week since it was last attended.
function memory.rank(row, now)
  local dwell = math.min((row.dwell or 0) / 60, 60)
  local signals = (row.visits or 0) + 3 * (row.edits or 0) + 2 * (row.yanks or 0) + dwell
  local days = math.max(0, now - (row.last or now)) / 86400
  return signals * 0.5 ^ (days / memory.half_life_days)
end

-- memory.boosts([kind], [limit]): the `kind` rows (files by default)
-- as a table of subject → boost, the best-ranked row 0.5 and the
-- rest in proportion; a pinned row 10 plus its place, so pins come
-- first in pin order.
function memory.boosts(kind, limit)
  local rows = kawoosh.memory { kind = kind or "file", limit = limit or 500 }
  local now = kawoosh.now()
  local ranks, max = {}, 0
  for i, r in ipairs(rows) do
    local s = memory.rank(r, now)
    ranks[i] = s
    if s > max then max = s end
  end
  local by = {}
  for i, r in ipairs(rows) do
    local b = max > 0 and (0.5 * ranks[i] / max) or 0
    if (r.pinned or 0) > 0 then b = 10 + 1 / r.pinned end
    by[r.subject] = b
  end
  return by
end
