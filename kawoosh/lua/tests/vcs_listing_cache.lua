-- A slow backend never removes the last colours while the listing is
-- refilled, and an invalidation during a read cannot publish that read.
local dir = os.tmpname() .. "-listing-cache"
local asks, probes, held = 0, 0, {}
kawoosh.vcs.register("listing-cache", {
  probe = function(d, done) probes = probes + 1; done(d) end,
  status = function(root, done, opts)
    asks = asks + 1
    kawoosh.test.eq(opts.under, dir)
    kawoosh.test.eq(opts.untracked, "normal")
    held[#held + 1] = done
  end,
})
kawoosh.opt("vcs.backends", { "listing-cache" })
kawoosh.frame()
local answers = {}
local function paint(files) answers[#answers + 1] = files[1] and files[1].state or "clean" end
kawoosh.vcs.listing_status(dir, paint)
kawoosh.vcs.listing_status(dir, paint)
kawoosh.test.eq(asks, 1, "concurrent refills share one ask")
held[1]({ { path = dir .. "/a", state = "ignored" } })
kawoosh.test.eq(answers[#answers], "ignored")
kawoosh.vcs.listing_status(dir, paint)
kawoosh.test.eq(answers[#answers], "ignored", "cached answer is synchronous")
kawoosh.test.eq(asks, 2, "refetch in background")
kawoosh.vcs.listing_changed(dir, true)
held[2]({ { path = dir .. "/a", state = "modified" } })
kawoosh.test.eq(asks, 3, "changed during a read: ask again")
kawoosh.test.eq(answers[#answers], "ignored", "superseded answer never paints")
held[3]({ { path = dir .. "/a", state = "untracked" } })
kawoosh.test.eq(answers[#answers], "untracked")
kawoosh.test.eq(probes, 2, "repository marker changes reprobe ownership")
kawoosh.vcs.moved(dir)
kawoosh.test.eq(asks, 4, "repository changes refresh listings")
held[4]({})
kawoosh.test.eq(answers[#answers], "clean", "fresh clean status clears old colours")
