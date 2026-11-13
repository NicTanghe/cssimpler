#set document(title: "PR #9: transition refresh performance", author: "cssimpler", date: datetime(year: 2026, month: 11, day: 13))
#set page(paper: "a4", margin: (x: 20mm, y: 16mm), numbering: none)
#set text(font: "Helvetica Neue", size: 9pt, fill: rgb("#1c2d3a"))
#set par(leading: 0.56em)
#let navy = rgb("#15344a")
#let teal = rgb("#087b7a")
#text(size: 9pt, weight: "bold", fill: teal, tracking: 0.8pt)[CSSIMPLER  /  PERFORMANCE NOTE]
#v(4pt)
#text(size: 22pt, weight: "bold", fill: navy)[What did PR \#9 cost?]
#v(4pt)
#text(size: 10pt, fill: rgb("#526776"))[Paint refreshes during layout transitions  ·  13 November 2026]
#v(8pt)
#block(width: 100%, inset: (x: 10pt, y: 8pt), radius: 4pt, fill: rgb("#e8f3f2"))[
  *Measured in a synthetic CPU test:* the fix raised median scene-preparation time by *19%* for full refreshes and *57-59%* for partial refreshes. It also allowed the animation to finish; the old version did not. These are not application frame rates.
]
#v(7pt)
#text(size: 11pt, weight: "bold", fill: navy)[Measured cost]
#v(4pt)
#text(size: 8pt, fill: rgb("#526776"))[Times are microseconds per frame; allocation is requested KiB per frame. Changes use unrounded values.]
#v(3pt)
#table(
  columns: (1.25fr, 0.55fr, 1fr, 1fr, 1fr, 1fr),
  inset: (x: 5pt, y: 5pt), align: (left, right, right, right, right, right),
  stroke: (x, y) => if y == 0 { none } else { (bottom: 0.35pt + rgb("#dfe7eb")) },
  fill: (x, y) => if y == 0 { rgb("#dce9ee") } else if calc.even(y) { rgb("#f5f8f9") } else { white },
  [*Refresh*], [*Nodes*], [*Median*\ *before → after*], [*P95*\ *before → after*], [*Change*\ *median*], [*Alloc. KiB*\ *before → after*],
  [Idle control], [100], [211 → 221], [276 → 306], [+4.8%], [1,566 → 1,566],
  [Full paint], [100], [202 → 239], [268 → 300], [+18.8%], [1,590 → 1,828],
  [Partial paint], [100], [88 → 137], [144 → 192], [+56.8%], [660 → 987],
  [Idle control], [500], [991 → 1,003], [1,149 → 1,166], [+1.2%], [6,812 → 6,812],
  [Full paint], [500], [994 → 1,183], [1,220 → 1,339], [+19.0%], [6,928 → 8,088],
  [Partial paint], [500], [419 → 666], [568 → 850], [+58.9%], [2,886 → 4,483],
)
#v(6pt)
#text(size: 11pt, weight: "bold", fill: navy)[What the comparison means]
#v(3pt)
In each active case, the switch was checked seven times per run after enough simulated time to finish: *before: 0/7; after: 7/7*, in all five runs. The earlier version repeatedly restarted the transition, so this is a correctness fix with a real cost, not an equal-work speed comparison. Partial refreshes now clone the destination scene while a transition is active; other transition bookkeeping also changes. Idle allocation counts and bytes were identical; the small idle timing differences may reflect noise.
#v(5pt)
#text(size: 11pt, weight: "bold", fill: navy)[Method and limits]
#v(3pt)
Apple M1 Pro, 16 GiB, macOS 27.0.1; Rust 1.99.1, release build, one process at a time. Five alternating runs per revision and size; each case warms up for 32 frames then measures 240 frames at simulated 16 ms intervals. The table reports the median of run medians and the median of run P95s. A switch toggles every 32 frames while a status label changes each frame; 100 or 500 additional elements make up the scene. The idle control changes the label without toggling.

`App::frame` includes scene preparation and its returned-scene clone. A counting allocator adds overhead; allocation is cumulative requested bytes, *not* peak memory. No pixels, window presentation, GPU, or real FoldCraft workload were measured. Desktop activity and unlocked clocks limit precision. Findings should guide a realistic follow-up, not predict production FPS.
#v(5pt)
#line(length: 100%, stroke: 0.5pt + rgb("#cbd8df"))
#v(3pt)
#text(size: 7.6pt, fill: rgb("#526776"))[Before `34472cb`  ·  After `ac4c2ee`  ·  #link("https://github.com/NicTanghe/cssimpler/pull/9")[PR \#9]  ·  Reproduce: `python3 docs/reports/pr9-paint-refresh-run.py --output /tmp/pr9-results.json`]
